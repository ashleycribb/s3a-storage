# ADR 0013: Unknown tiles and reserved bytes: rules and precedent

Status: Accepted — 2026-10-21. Builds on ADR-0011. No on-disk format or seal-hash change.

## Context
`hypertile-future-proof.md` listed two spec gaps a second implementation would trip over:

1. **Unknown tiles.** The rules disagreed on whether an unknown tile without `COPY_THROUGH_COMPACT` survives a rewrite. ADR-0011 already fixed the text and code (always copy through). The precedent wasn't written down, and no conformance vector checked it.
2. **Non-zero reserved bytes.** TILE-SPEC §3.3 says readers ignore reserved bytes. But the file-header table called `flags` (bytes 6..8) and `_reserved1` (bytes 20..24) "reserved; MUST be 0", while the Edge profile and the code use them as feature flags and as the tile stride. Code accepted any `flags` value and any non-zero stride.

## Precedent (sources fetched 2026-10-21)
- **PNG, 3rd edition** (W3C): §5.4 https://www.w3.org/TR/png-3/#5Chunk-naming-conventions, §14.2 https://www.w3.org/TR/png-3/#14Ordering
  - §5.4, Table 6: ancillary bit: "A decoder trying to extract the image, upon encountering an unknown chunk type in which the ancillary bit is 0, shall indicate to the user that the image contains information it cannot safely interpret." … "A decoder encountering an unknown chunk type in which the ancillary bit is 1 can safely ignore the chunk".
  - Safe-to-copy bit: "This bit defines the proper handling of unrecognized chunks in a datastream that is being modified."
  - §14.2 (Behavior of PNG editors): "If a chunk's safe-to-copy bit is 1, the chunk may be copied to a modified PNG datastream whether or not the PNG editor recognizes the chunk type, and regardless of the extent of the datastream modifications. If a chunk's safe-to-copy bit is 0, it indicates that the chunk depends on the image data."
- **RFC 9293** (TCP), §3.1, https://www.rfc-editor.org/rfc/rfc9293#section-3.1: "Reserved (Rsrvd): … Must be zero in generated segments and must be ignored in received segments if the corresponding future features are not implemented by the sending or receiving host."
- **RFC 9413** (Maintaining Robust Protocols):
  - §2.2, https://www.rfc-editor.org/rfc/rfc9413#section-2.2: "A well-designed extensibility mechanism establishes clear rules for the handling of elements like new messages or parameters."
  - §5.1, https://www.rfc-editor.org/rfc/rfc9413#section-5.1 ("Virtuous Intolerance"): "Choosing to generate fatal errors for unspecified conditions instead of attempting error recovery can ensure that faults receive attention."

## Decision
1. **Unknown tile types are ancillary and safe to copy.**
   - Readers skip them (§7.2.1, §829).
   - Rewriters copy them through with the payload byte-for-byte (ADR-0011).
   - S3A has no on-disk critical or unsafe-to-copy bit and doesn't add one. Instead, any type whose content is derived from other tiles must be a registered (known) type, so tools can drop and regenerate it (tile 83 is the existing example). This is PNG's model with "unsafe" limited to known types.
2. **Reserved bytes (no meaning in this version): writers zero them, readers ignore them** (RFC 9293 §3.1). This applies to tile-header `_reserved*`, padding, and file-header block bytes 128..512. Validators MAY warn, but MUST NOT fail.
3. **Layout-affecting header fields fail closed** (PNG's critical case; RFC 9413 §5.1).
   - `FileHeader.flags` may only use the defined bits `0x001F`.
   - `FileHeader._reserved1` (`tile_stride`) must be 0, 4096 or 8192.
   - Anything else is rejected after slot arbitration, so an unknown flag in the newest slot can't cause a silent fallback to an older generation.

## Code changes
- `s3a_core::KNOWN_HEADER_FLAGS`, `VALID_TILE_STRIDES`, `FileHeader::check_layout_fields()`, and the new `S3AError::UnsupportedHeaderFlags(u16)`. A bad stride uses the existing `InvalidTileSize`.
- `s3a_engine::resolve_active_header` calls `check_layout_fields()` on the chosen slot.
- Before this change, a file with flag `0x0100` opened as `ok` (checked against an `origin/main` build). After it, the file is rejected.

## Consequences
- Files written by this codebase are unaffected: they only use defined flags and strides. `cargo test --workspace` passes unchanged.
- `conformance/vectors-1.1` (a superset of 1.0) adds 6 cases: `reserved_tile_header_nonzero`, `reserved_file_header_padding_nonzero`, `header_unknown_flag_bit`, `header_invalid_tile_stride`, `header_unknown_flag_newest_slot_only`, and writer-level `unknown_tile_rewrite`.
- Defining a new flag bit later is a spec change that old readers deliberately refuse. That is the intended trade-off for fields that change parsing.
