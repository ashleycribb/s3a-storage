# 2026-10-21 — Unknown tiles copied through by default; COMPATIBILITY.md

**What changed.**
- Spec: TILE-SPEC §7.2 now says rewrite tools MUST copy unknown tile types through intact whether
  or not they carry `COPY_THROUGH_COMPACT`; dropping needs an explicit user request (ADR-0011).
- Code: `TileFusionEngine::fuse_telemetry_tiles` previously dropped unknown tile types; it now
  copies them into the output with payload bytes unchanged (new `file_uuid`/`tile_id`).
  `Compactor::compact` and `migrate` already preserved them; unchanged.
- New `COMPATIBILITY.md` with the 1.x format compatibility promise.

**Who is affected.** Callers of tile fusion whose inputs contain unregistered tile types will see
those tiles in the output (higher `tile_count`). No on-disk format change.
