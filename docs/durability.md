# S3A Durability & Crash Recovery Contract

This specification documents the S3A dual-generation header architecture, append ordering barriers, compaction replacement protocol, and crash recovery guarantees under unexpected process termination and system restarts.

---

## 1. Dual-Slot A/B Header Model

Every S3A archive begins with a fixed 512-byte `DualFileHeader` structure aligned to 8-byte boundaries.

```
+-------------------------------------------------------------------------------+
|                           S3A Archive File Layout                             |
+-------------------------------------------------------------------------------+
| Byte 0..63    : Slot A (64-byte FileHeader)                                   |
| Byte 64..127  : Slot B (64-byte FileHeader)                                   |
| Byte 128..511 : Reserved Padding Zeroes (384 bytes)                           |
| Byte 512..    : Hyper-Tiles (128 KB each, page-aligned)                       |
+-------------------------------------------------------------------------------+
```

### Slot Structure (`FileHeader`, 64 bytes)
- `magic`: `[u8; 4]` (`S3A\0` or `0x00413353`)
- `version`: `u16` (current = 1)
- `flags`: `u16` (status and encryption bits)
- `generation`: `u64` (monotonically increasing commit counter)
- `tile_count`: `u32` (number of fully committed 128 KB hyper-tiles)
- `created_timestamp`: `u64` (seconds since Unix epoch)
- `commit_timestamp`: `u64` (seconds since Unix epoch of this commit)
- `header_crc32`: `u32` (hardware CRC32C of the 64-byte slot excluding `header_crc32`)
- `addon_mask`: `u32` (capability bitfield e.g. `0x0001` for `ADDON_EDU`)
- Reserved: 20 bytes

### Active Slot Resolution Algorithm (`resolve_active_header`)
On archive open or recovery:
1. Verify Slot A: check magic == `S3A\0`, version == 1, and `header_crc32 == compute_header_crc32(slot_a)`.
2. Verify Slot B: check magic == `S3A\0`, version == 1, and `header_crc32 == compute_header_crc32(slot_b)`.
3. Decision matrix:
   - **Both valid**: Select slot with highest `generation`. If generations match, Slot A wins.
   - **Slot A valid, Slot B invalid**: Select Slot A.
   - **Slot B valid, Slot A invalid**: Select Slot B.
   - **Both invalid**: Return `S3AError::CorruptedHeader` (fail-closed to prevent silent data corruption).

---

## 2. Append Ordering & Data Flush Barriers

All Hyper-Tile writes follow a strict three-phase commit sequence:

```
[Phase 1: Write Data]       [Phase 2: Data Sync Barrier]      [Phase 3: Header Slot Commit]
Write 128 KB tile bytes  ->  file.flush()                 ->   Write alternate slot
(Header + payload + pad)     file.sync_data()                  (Next gen, tile_count + 1)
                                                               file.flush()
                                                               file.sync_all()
```

### Guarantees:
- Tile payloads are physically synced to underlying storage via `file.sync_data()` before the file header slot is updated.
- If a crash occurs before Phase 3 completes, the active header still points to the prior committed `tile_count`, rendering the partial or uncommitted tile bytes invisible to readers.

---

## 3. Recovery Protocol (`recover_archive` & `TileWriter::open_append`)

When opening an existing archive for read, append, or recovery:
1. **Header Slot Evaluation**: Run `resolve_active_header` to find the active valid generation and committed `tile_count`.
2. **Torn EOF Truncation**:
   - Compute expected archive file size: `expected = 512 + committed_tile_count * 131,072`.
   - If actual `file_len > expected`: The file suffered an interrupted tile write or crash before header commit. The recovery routine calls `file.set_len(expected)`, followed by `file.flush()` and `file.sync_all()`.
3. **Orphan Compaction Temp Cleanup**:
   - Call `clean_orphan_temps(path)` to remove abandoned compaction and swap artifacts (`*.compact.*`, `*.tmp.*`, `*.old.*`).
4. **Append Resumption**:
   - Seek to `expected` EOF.
   - Next append targets the alternate header slot with `generation = active_generation + 1`.

---

## 4. Compaction & Atomic Replacement Protocol

Compaction merges and purges tombstones without in-place mutation of the live archive:

```
[Step 1: Generate Temp Path]
unique_compact_temp_path(&live_path)
-> live.compact.{pid}.{unix_nano}_{nonce}.s3a

[Step 2: Compact to Temp File]
Compactor::compact(&[input_paths], &temp_path)
- Telemetry tombstones purged via O(N) HashMap deduplication
- Domain pack tiles (10..=15) and unspecialized tiles copied through intact
- addon_mask bitwise OR accumulated onto temp archive
- sync_all() called on completed temp archive

[Step 3: Atomic Replace]
atomic_replace(&temp_path, &live_path)
- Platform rename (MOVEFILE_REPLACE_EXISTING on Windows)
- With backup swap fallback (live -> old.tmp -> new -> live) if target locked
- Sync parent directory / live file handle

[Step 4: Cleanup on Failure]
If compaction or replacement fails, temp file is unlinked immediately.
Live archive remains entirely untouched and readable.
```

---

## 5. Crash Outcome Matrix

The following fault-injection scenarios are verified in `tests/durability_crash_harness.rs`:

| Scenario | Injected Failure State | Recovery Action | Post-Recovery Guarantees |
|:---|:---|:---|:---|
| **Active Slot Corrupted** | 4 bytes zeroed/scrambled in active Slot A header (CRC invalid) | `resolve_active_header` falls back to Slot B | Reader sees Generation N-1; append overwrites corrupt Slot A as Generation N+1 |
| **Torn EOF Partial Tile** | Process killed after appending 40 KB of a 128 KB tile | `open_append` / `recover_archive` truncates file to `512 + N * 131072` | Torn bytes removed; file size exact; checksums pass; appends resume safely |
| **Uncommitted Full Tile** | 128 KB tile written + synced, but process killed before header slot commit | Active header reflects prior `tile_count = N`; recovery truncates uncommitted tile | Reader sees N tiles; no ghost tiles; appends cleanly overwrite uncommitted slot |
| **Mid-Compact Abandoned Temp** | Compactor crashes while writing `foo.compact.<pid>.<ts>.s3a` | `open_append` / `recover_archive` cleans orphan temps matching `{stem}.compact.*` | Live archive `foo.s3a` unaffected; disk space reclaimed; reader intact |
| **Atomic Replace Success** | Compaction succeeds and replaces live file atomically | Atomically renames temp -> live with sync barrier | Tombstones purged; only active records visible; no orphan temporary files |
| **Pack Tile & Mask Preservation** | Compacting archive with Scholar Pack tiles (10..=15) + external pack tiles | Copy-through preserves pack tiles and tombstones; OR-accumulates `addon_mask` | `ADDON_EDU` and other pack masks preserved; pack records intact; checksums pass |

---

## 6. Scope Boundaries & Non-Goals (Phase 1)

> [!NOTE]
> S3A provides **single-writer crash resilience** via dual-generation slots and write barriers. This is a robust append/compaction durability harness, not a full distributed transactional database.

### What Phase 1 Durability Covers:
- Process crash, `SIGKILL`, or OS abnormal termination during writes or compaction.
- Torn writes on append and graceful truncation back to the last atomic commit.
- Active header slot corruption recovery via redundant alternate slot.
- Atomic file replacement for compaction and background garbage collection.

### Non-Goals (Explicitly Out of Scope for Phase 1):
- **Multi-writer concurrent locking**: S3A assumes a single writer process (or coordinator) at a time per archive.
- **Physical drive power-loss without write cache flush**: Physical disks with volatile, unbuffered write caching disabled may reorder underlying controller buffers unless enterprise write-through drives are used.
- **ACID Distributed Transactions**: S3A does not claim multi-file distributed 2PC or distributed WAL.
- **Undo logging for partially overwritten files**: S3A uses append-only storage and atomic file replacement; it never writes record data in-place.
