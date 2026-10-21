# 2026-10-21 — Tile fusion keeps known non-telemetry tiles

**What changed.** `TileFusionEngine::fuse_telemetry_tiles` used to drop every tile that was not
telemetry (schema, marks, consent, erasure, blob, pack tiles...). It now carries them through with
payload bytes unchanged (new `file_uuid` / `tile_id`), per ADR-0011's no-silent-loss rule.
Telemetry is still re-chunked, deduplicated and tombstone-filtered as before.

**Who is affected.** Fusion outputs now contain those tiles (higher `tile_count`). Fusion does not
apply erasure/consent semantics or remap tile-id references (marks pointing at old telemetry tile
ids); use `Compactor::compact` when those must be honored. No on-disk format change.

Also: duplicate TILE-SPEC headings renumbered (§3.3/§3.4 duplicates → §3.5–§3.8).
