# API change: `fsutil::atomic_replace` propagates sync errors (A5)

- Function: `s3a_engine::fsutil::atomic_replace(from, to) -> io::Result<()>`
- Signature: unchanged.
- Behavior change:
  - Returns `Err` if the source file cannot be opened for syncing or `sync_all()` fails
    (previously ignored). Destination is untouched in that case.
  - On Unix, fsyncs the parent directory after the rename and returns `Err` if that
    fails (previously not done). The destination already holds the new contents.
  - Windows: unchanged (TODO: `MOVEFILE_WRITE_THROUGH`).
- On-disk format: no change. Backward compatible for all existing `.s3a` files.
- See `docs/adr/0001-atomic-replace-durability.md`.
