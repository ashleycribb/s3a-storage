# ADR 0011: Unknown tiles are copied through by default

Status: Accepted — 2026-10-21

## Context
TILE-SPEC §7.2 only required a compactor to keep an unknown tile when it carried
`TileTypeFlags::COPY_THROUGH_COMPACT`, and said nothing about unflagged unknown tiles. But a tool
cannot read flags from a registry entry for a type it does not know, so the rule was unusable and
left a silent data-loss path. In code, `Compactor` and `migrate` already preserved all unknown
tiles, while `TileFusionEngine` silently dropped every non-telemetry tile.

## Decision
- Every rewrite tool (compaction, fusion, migration, repair) copies unknown tile types through with
  the payload byte-for-byte, regardless of flags. Only writer-owned relocation fields change.
- Dropping unknown tiles requires an explicit user request; none of the current tools offers one.
- Erasure still applies: erased record bytes are zeroed.
- `COPY_THROUGH_COMPACT` stays as a hint for known types.

## Consequences
- `TileFusionEngine::fuse_telemetry_tiles` now carries every non-telemetry tile (known or unknown)
  through unchanged; it only re-encodes telemetry.
- New test `crates/s3a-engine/tests/unknown_tile_copy_through.rs` checks an unflagged unknown tile
  survives `Compactor::compact` and tile fusion.
