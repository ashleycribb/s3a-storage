# S3A Production Operations Manual & DBA Runbook

This guide details operational commands, health diagnostics, crash recovery procedures, and machine-readable interfaces for database administrators managing S3A archives.

---

## 1. Stable Operations Toolbox Overview

The `s3a-cli` utility organizes commands into clean functional domains. Production DBA workflows rely on the **Stable Core Ops & Maintenance** toolset:

```text
STABLE CORE OPS & MAINTENANCE:
  s3a-cli inspect [--json] <archive>                Inspect Hyper-Tile layout, headers, and metadata
  s3a-cli tour [--json] [--floor N] <archive>        Tour archive: lobby, floors, array slice, cross-section, history
  s3a-cli verify [--json] <archive>                 Verify all CRC32/CRC32C checksums in archive
  s3a-cli status [--json] <archive>                 Quick archive state, generation, slot, and addon mask
  s3a-cli doctor [--json] <archive>                 Deep integrity diagnosis (A/B slots, torn bytes, orphan temps)
  s3a-cli recover <archive>                         Crash recovery: truncate torn bytes, clean orphan temps, harden A/B slots
  s3a-cli compact <out_file> <in_file1> [in_file2...] Compact files into stratified Hyper-Tiles
  s3a-cli fuse <out_file> <in_file1> [in_file2...]  Fuse multi-tile archives into a consolidated Hyper-Tile archive
  s3a-cli addon list [archive]                      List active modular domain addons on an archive
  s3a-cli addon enable <archive> <addon_name>       Apply a modular addon (scholar, robotics, gis, da, wearable, ontology)
  s3a-cli query [--json] "<S3A_QL_STATEMENT>\"       Execute versioned S3A-QL v1 statement
  s3a-cli snapshot export <live_db> <out> [uuid]    Export portable Scholar Research Snapshot container
  s3a-cli snapshot intake <snapshot> [--into <live>] Intake & validate portable SRS snapshot into live database
  s3a-cli snapshot inspect <snapshot>               Inspect SRS manifest header, sector CRC32, and inventory
```

---

## 2. Command Reference

### 2.1 `status`
Quick, non-intrusive status check of committed metadata, active generation, dual-slot state, and enabled domain addon capabilities.

```bash
# Human-readable output
s3a-cli status live_production.s3a

# Machine-readable output for monitoring agents
s3a-cli status --json live_production.s3a
```

#### JSON Output:
```json
{
  "ok": true,
  "file": "live_production.s3a",
  "size_bytes": 528384,
  "version": 1,
  "generation": 42,
  "active_slot": "B",
  "header_crc32": "0x3F81A9B0",
  "tile_count": 4,
  "addon_mask": "0x0005",
  "active_addons": ["scholar", "robotics"]
}
```

---

### 2.2 `doctor`
Deep diagnostic check analyzing:
1. **Header Slot Integrity**: Evaluates Slot A and Slot B hardware CRC32C checksums independently.
2. **Active Generation Resolution**: Detects if one slot experienced a torn write mid-commit.
3. **File Length Alignment**: Compares actual file size against expected `4096 + tile_count * 131072` bytes. Detects uncommitted torn trailing writes or unexpected truncation.
4. **Orphan Temporary Files**: Identifies abandoned `{stem}.compact.*` or `{stem}.tmp.*` files left behind by ungraceful system reboots.
5. **Hyper-Tile Checksums**: Validates every committed tile payload CRC32C checksum and records layout version compliance.

```bash
s3a-cli doctor production.s3a
s3a-cli doctor --json production.s3a
```

#### Health States:
- **`HEALTHY`**: Both slots valid, zero torn bytes, zero orphan temps, all tile checksums match.
- **`DEGRADED`**: Exactly one slot valid, torn trailing bytes detected, or orphan temps found. Archive is fully readable; run `s3a-cli recover` to harden.
- **`CORRUPTED`**: Both slots corrupted, file truncated before committed tile count, or tile payload CRC mismatches.

#### JSON Output:
```json
{
  "ok": true,
  "file": "production.s3a",
  "health": "DEGRADED",
  "slot_a": { "valid": true, "generation": 10 },
  "slot_b": { "valid": false, "generation": 11 },
  "active_slot": "A",
  "tile_count": 5,
  "torn_bytes": 65536,
  "truncated_bytes": 0,
  "orphan_temps": 1,
  "tile_crc_errors": 0
}
```

---

### 2.3 `recover`
Deterministic crash recovery and archive hardening:
1. Validates dual A/B header slots and locks onto the highest valid committed generation.
2. Truncates uncommitted partial tile bytes appended past the active generation boundary.
3. Removes orphan compaction files (`*.compact.*`, `*.tmp.*`).
4. Re-synchronizes file headers to ensure pristine dual-slot redundancy.

```bash
s3a-cli recover production.s3a
```

Output:
```text
Initiating S3A Crash Recovery & Hardening on 'production.s3a'...
✓ Recovery SUCCESS:
  Committed Generation: 10
  Committed Tiles:      5
  Header CRC32:         0x5A18C3E1
  Addon Mask:           0x0001
Archive is hardened, verified, and ready for queries.
```

---

### 2.4 `verify` & `inspect`
- **`verify [--json] <archive>`**: Runs fast SIMD-accelerated CRC32C checksum passes across all tile payloads and headers.
- **`inspect [--json] <archive>`**: Dumps metadata for every Hyper-Tile: type, layout version, timestamp range, bounding hulls, Bloom filter fill ratio, and tombstone statistics.

```bash
s3a-cli verify production.s3a
s3a-cli inspect --json production.s3a
```

---

### 2.5 `compact` & `fuse`
- **`compact <out_file> <in_files...>`**: Purges tombstones, reorganizes tiles into stratified tiers, and recomputes learned index splines.
- **`fuse <out_file> <in_files...>`**: Consolidates separate append-only delta archives into an authoritative archive while preserving capability bitmasks.

---

### 2.6 `addon`
Manages modular capability bits in the archive header.
```bash
# List active domain addons
s3a-cli addon list archive.s3a

# Enable a domain pack capability
s3a-cli addon enable archive.s3a scholar
s3a-cli addon enable archive.s3a robotics
s3a-cli addon enable archive.s3a gis
s3a-cli addon enable archive.s3a da
```

---

## 3. Incident Runbook for DBAs

### Scenario A: Power Loss During Ingest / Append
1. **Symptom**: Monitoring reports file size greater than expected or process crashed during ingestion.
2. **Action**:
   ```bash
   s3a-cli doctor live.s3a
   ```
   If doctor reports `health: DEGRADED` with `torn_bytes > 0`:
   ```bash
   s3a-cli recover live.s3a
   ```
3. **Verification**:
   ```bash
   s3a-cli verify live.s3a
   ```

### Scenario B: Compaction Host Rebooted Mid-Job
1. **Symptom**: Host rebooted while `Compactor` or `BackgroundCompactor` was running. Temporary `.compact.*` files remain on disk.
2. **Action**:
   ```bash
   s3a-cli doctor live.s3a
   ```
   Notice `orphan_temps > 0`. Run:
   ```bash
   s3a-cli recover live.s3a
   ```
   Orphan temp files are safely unlinked and removed.
3. **Restart Compaction**:
   ```bash
   s3a-cli compact live.s3a live.s3a
   ```
