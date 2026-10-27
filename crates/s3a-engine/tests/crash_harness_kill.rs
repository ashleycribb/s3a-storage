//! Crash-test harness (durability audit fix #3).
//!
//! 1. Kill-at-random: a child process appends tiles in a loop and is SIGKILLed at seeded
//!    random points; the archive must reopen with every acknowledged tile intact and all
//!    CRCs passing, then accept further appends.
//! 2. Torn tail: truncate/garble the file at seeded random offsets past the committed size;
//!    `open_append` must cut back to the committed state.
//! 3. Torn header: partially write the next header slot (byte prefixes, and 512-byte /
//!    4 KiB sector-sized tears) and assert A/B recovery picks the valid slot.
//! 4. Legacy regression: `record_layout_version == 0` tile + unhashed slot A + zeroed slot B
//!    still opens after #161.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rand::{rngs::StdRng, Rng, SeedableRng};
use s3a_core::{
    FileHeader, HyperTileHeader, FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET,
    FILE_HEADER_SLOT_B_OFFSET, FILE_HEADER_SLOT_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::{MmapReader, TelemetryRecord, TileType, TileWriter};

const CHILD_ENV: &str = "S3A_CRASH_HARNESS_CHILD";
const SEED: u64 = 0x53A_C4A5;
static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "s3a_crashh_{}_{}_{}.s3a",
            tag,
            std::process::id(),
            id
        ));
        let _ = fs::remove_file(&p);
        Tmp(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = s3a_engine::clean_orphan_temps(&self.0);
        let _ = fs::remove_file(&self.0);
    }
}

fn rec(i: u64) -> TelemetryRecord {
    TelemetryRecord::new(1_000 + i, 7, 3, i as f64)
}

fn append(w: &mut TileWriter, i: u64) {
    let r = rec(i);
    w.write_hyper_tile(TileType::TELEMETRY, &[r], Some(&[r.timestamp]), None)
        .unwrap();
}

/// Reopen, verify CRCs, and check tile `i` holds record `i` for every committed tile.
fn assert_intact(path: &Path, min_tiles: u32) -> u32 {
    let r = MmapReader::open(path).expect("archive must reopen");
    r.verify_checksums().expect("no silent corruption");
    let n = r.tile_count();
    assert!(n >= min_tiles, "lost acknowledged tiles: {n} < {min_tiles}");
    for i in 0..n as usize {
        let (_, payload) = r.get_tile(i).unwrap();
        let got: TelemetryRecord =
            bytemuck::pod_read_unaligned(&payload[..std::mem::size_of::<TelemetryRecord>()]);
        assert_eq!(got.timestamp, rec(i as u64).timestamp, "tile {i} content");
    }
    n
}

/// Child body: only active when re-executed by the parent with CHILD_ENV set.
#[test]
fn crash_child_append_loop() {
    let Ok(path) = std::env::var(CHILD_ENV) else {
        return;
    };
    let mut w = if Path::new(&path).exists() {
        TileWriter::open_append(&path).unwrap()
    } else {
        TileWriter::create(&path).unwrap()
    };
    let mut i = w.tile_count() as u64;
    let out = std::io::stdout();
    loop {
        append(&mut w, i);
        i += 1;
        // Ack only after write_hyper_tile returned (tile + header slot synced).
        writeln!(out.lock(), "ACK {i}").unwrap();
    }
}

#[cfg(unix)]
#[test]
fn kill_child_at_random_points_keeps_committed_tiles() {
    use std::process::{Command, Stdio};
    if std::env::var(CHILD_ENV).is_ok() {
        return;
    }
    let iters: u32 = std::env::var("S3A_CRASH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    let mut rng = StdRng::seed_from_u64(SEED);
    let exe = std::env::current_exe().unwrap();
    let t = Tmp::new("kill");
    let mut acked = 0u32;
    for round in 0..iters {
        let mut child = Command::new(&exe)
            .args([
                "--exact",
                "crash_child_append_loop",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, t.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut last = 0u32;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(n) = line.strip_prefix("ACK ") {
                    last = n.trim().parse().unwrap();
                }
            }
            last
        });
        std::thread::sleep(std::time::Duration::from_millis(rng.gen_range(5..60)));
        child.kill().unwrap(); // SIGKILL: no destructors
        child.wait().unwrap();
        let last = reader.join().unwrap();
        acked = acked.max(last);
        let n = assert_intact(t.path(), acked);
        // At most one tile may be committed-but-unacked (killed between sync and println).
        assert!(
            n <= acked + 1,
            "round {round}: {n} tiles > acked {acked} + 1"
        );
        acked = n;
    }
    // Writer still works after all the crashes.
    let mut w = TileWriter::open_append(t.path()).unwrap();
    append(&mut w, acked as u64);
    drop(w);
    assert_intact(t.path(), acked + 1);
    eprintln!(
        "kill harness: {iters} rounds, {} tiles committed",
        acked + 1
    );
}

#[test]
fn torn_tail_at_random_offsets_is_truncated_to_committed() {
    let mut rng = StdRng::seed_from_u64(SEED ^ 1);
    let base = Tmp::new("tail_base");
    let mut w = TileWriter::create(base.path()).unwrap();
    for i in 0..3 {
        append(&mut w, i);
    }
    drop(w);
    let committed = fs::read(base.path()).unwrap();
    assert_eq!(committed.len(), FILE_HEADER_SIZE + 3 * HYPER_TILE_SIZE);
    for _ in 0..32 {
        let t = Tmp::new("tail");
        // Simulate a crashed 4th append: some prefix of a tile (maybe garbage) lands.
        let extra = rng.gen_range(1..=HYPER_TILE_SIZE);
        let mut data = committed.clone();
        let mut tail = vec![0u8; extra];
        if rng.gen_bool(0.5) {
            rng.fill(&mut tail[..]);
        }
        data.extend_from_slice(&tail);
        fs::write(t.path(), &data).unwrap();
        // Readers ignore the uncommitted tail; append truncates it.
        assert_intact(t.path(), 3);
        let mut w = TileWriter::open_append(t.path()).unwrap();
        assert_eq!(w.tile_count(), 3);
        append(&mut w, 3);
        drop(w);
        assert_eq!(assert_intact(t.path(), 4), 4);
        assert_eq!(
            fs::metadata(t.path()).unwrap().len() as usize,
            FILE_HEADER_SIZE + 4 * HYPER_TILE_SIZE
        );
    }
}

/// Builds an archive with two committed generations so both slots hold valid headers,
/// and returns (bytes, active slot offset, inactive slot offset, committed tiles).
fn two_gen_archive() -> (Vec<u8>, usize, usize, u32) {
    let t = Tmp::new("hdr_base");
    let mut w = TileWriter::create(t.path()).unwrap();
    append(&mut w, 0);
    append(&mut w, 1);
    drop(w);
    let r = MmapReader::open(t.path()).unwrap();
    let (act, inact) = if r.active_slot() == 0 {
        (FILE_HEADER_SLOT_A_OFFSET, FILE_HEADER_SLOT_B_OFFSET)
    } else {
        (FILE_HEADER_SLOT_B_OFFSET, FILE_HEADER_SLOT_A_OFFSET)
    };
    let n = r.tile_count();
    drop(r);
    (fs::read(t.path()).unwrap(), act, inact, n)
}

fn slot(data: &[u8], off: usize) -> FileHeader {
    bytemuck::pod_read_unaligned(&data[off..off + FILE_HEADER_SLOT_SIZE])
}

#[test]
fn torn_header_slot_prefix_writes_recover_to_valid_slot() {
    let (data, act, inact, n) = two_gen_archive();
    let gen = slot(&data, act).generation;
    // The next commit would write gen+1 into the inactive slot. Tear it at every byte.
    let mut next = slot(&data, act);
    next.generation = gen + 1;
    next.tile_count = n + 1;
    next.header_crc32 = s3a_engine::compute_header_crc32(&next);
    let new_bytes = bytemuck::bytes_of(&next).to_vec();
    for cut in 0..FILE_HEADER_SLOT_SIZE {
        let t = Tmp::new("hdr_prefix");
        let mut d = data.clone();
        d[inact..inact + cut].copy_from_slice(&new_bytes[..cut]);
        // Tile n+1 never landed in full, so the file stays at n tiles.
        fs::write(t.path(), &d).unwrap();
        let r = MmapReader::open(t.path()).unwrap();
        if d[inact..inact + FILE_HEADER_SLOT_SIZE] == new_bytes[..] {
            // Every remaining byte already matched: the new slot is complete and valid. Its
            // tile never landed, so reads must fail closed (SlotPastEof), not return garbage.
            assert_eq!(
                (r.generation(), r.tile_count()),
                (gen + 1, n + 1),
                "cut {cut}"
            );
            assert!(r.verify_checksums().is_err(), "cut {cut}: must fail closed");
            continue;
        }
        assert_eq!((r.generation(), r.tile_count()), (gen, n), "cut {cut}");
        r.verify_checksums().unwrap();
        drop(r);
        assert_intact(t.path(), n);
    }
}

#[test]
fn torn_header_at_512_and_4k_boundaries_recovers() {
    let (data, act, _inact, n) = two_gen_archive();
    let gen = slot(&data, act).generation;
    // Sector-sized tears: header region (512 B) or first 4 KiB page replaced by zeros or
    // by stale content; also a tear that zeroes only the active slot.
    let t = Tmp::new("hdr_sector");
    let lose_active = {
        let mut d = data.clone();
        d[act..act + FILE_HEADER_SLOT_SIZE].fill(0);
        d
    };
    fs::write(t.path(), &lose_active).unwrap();
    let r = MmapReader::open(t.path()).unwrap();
    assert_eq!(r.generation(), gen - 1, "falls back to older valid slot");
    r.verify_checksums().unwrap();
    drop(r);

    for boundary in [512usize, 4096] {
        // Both slots live inside the first 512-byte sector, so a torn sector that zeroes
        // it must fail closed rather than open garbage.
        let mut d = data.clone();
        let end = boundary.min(d.len());
        d[..end].fill(0);
        fs::write(t.path(), &d).unwrap();
        assert!(
            MmapReader::open(t.path()).is_err() || TileWriter::open_append(t.path()).is_err(),
            "zeroed first {boundary} bytes must fail closed"
        );
        assert!(
            MmapReader::open(t.path()).is_err(),
            "{boundary}: reader must fail closed"
        );

        // A torn sector inside committed tile 0's records (header intact) must be caught
        // by tile CRCs. (Records start at 1024 = file header + tile header.)
        let mut d = data.clone();
        let start = FILE_HEADER_SIZE + 512;
        d[start..start + 32].iter_mut().for_each(|b| *b ^= 0xFF);
        fs::write(t.path(), &d).unwrap();
        let r = MmapReader::open(t.path()).unwrap();
        assert_eq!((r.generation(), r.tile_count()), (gen, n));
        assert!(
            r.verify_checksums().is_err(),
            "{boundary}: damage inside committed tile must be detected"
        );
    }
}

#[test]
fn legacy_v0_tile_unhashed_slot_a_zero_slot_b_still_opens() {
    let t = Tmp::new("legacy");
    let mut w = TileWriter::create(t.path()).unwrap();
    append(&mut w, 0);
    drop(w);
    let mut d = fs::read(t.path()).unwrap();
    let active = MmapReader::open(t.path()).unwrap().active_slot();
    let src = if active == 0 {
        FILE_HEADER_SLOT_A_OFFSET
    } else {
        FILE_HEADER_SLOT_B_OFFSET
    };
    // Pre-A/B single-slot header: active header in slot A with CRC unset, slot B zeroed.
    let mut a = slot(&d, src);
    a.header_crc32 = 0;
    d[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]
        .copy_from_slice(bytemuck::bytes_of(&a));
    d[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE].fill(0);
    // Legacy tile: record_layout_version 0 and unset header CRC.
    let lv = FILE_HEADER_SIZE + offset_of!(HyperTileHeader, record_layout_version);
    let hc = FILE_HEADER_SIZE + offset_of!(HyperTileHeader, header_crc32);
    d[lv..lv + 4].fill(0);
    d[hc..hc + 4].fill(0);
    fs::write(t.path(), &d).unwrap();

    let r = MmapReader::open(t.path()).expect("legacy file must open after #161");
    assert_eq!((r.tile_count(), r.active_slot()), (1, 0));
    r.verify_checksums().unwrap();
    drop(r);
    // And it can still be appended to (writes a proper A/B header going forward).
    let mut w = TileWriter::open_append(t.path()).unwrap();
    append(&mut w, 1);
    drop(w);
    let r = MmapReader::open(t.path()).unwrap();
    assert_eq!(r.tile_count(), 2);
    r.verify_checksums().unwrap();
}
