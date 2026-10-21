# ADR 0001: Durability of `atomic_replace`

- Status: Proposed (roadmap item A5)
- Date: 2026-10-21
- Scope: `crates/s3a-engine/src/fsutil.rs::atomic_replace`

## Context

`atomic_replace(from, to)` swaps a freshly written temp file (e.g. a compaction
output) over the live `.s3a` archive. Before this change it:

1. called `sync_all()` on the source but discarded the result (`let _ = f.sync_all()`),
   and silently skipped the sync entirely if the file could not be opened;
2. renamed the file into place but never fsynced the parent directory.

On POSIX, `rename(2)` is atomic with respect to other processes, but the new
directory entry is only durable once the *directory* is fsynced. After a power
cut right after the rename, the filesystem may come back with the old entry (the
swap is undone) or, if the data fsync silently failed, a target pointing at
unflushed data.

## Decision

1. Open and `sync_all()` the source; **any error is returned** and the
   destination is not touched (the source is left in place for cleanup by
   `clean_orphan_temps`).
2. Rename into place (unchanged).
3. On Unix, open the parent directory of `to` (or `.`) and `sync_all()` it;
   **any error is returned**. At that point the rename has already happened, so
   the destination contains the complete new file; the error tells the caller
   the swap may not survive power loss.
4. Windows: behavior unchanged (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`
   via `fs::rename`, with backup-swap fallback). TODO: add
   `MOVEFILE_WRITE_THROUGH`; Windows has no portable directory fsync.

## Consequences

- Callers that previously "succeeded" despite a failed fsync now receive an
  `io::Error`. This is intended: a failed save must fail loudly.
- No on-disk format change. Files written before and after are byte-identical.
- Slight extra latency per replace (one directory fsync).

## Verification

- Unit tests with an injected sync function: file-sync failure leaves the
  destination untouched; directory-sync failure is propagated.
- Crash-injection test: a child process loops write-temp → `atomic_replace`
  and is SIGKILLed at varied points across 20 rounds; the target is always
  entirely the old or entirely the new payload, never torn.
- Not covered: true power loss (requires a VM/dm-flakey style harness).
