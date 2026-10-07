# S3A Schema Evolution & Tile Registry Specification

This document defines the schema evolution contract, tile layout versioning, and registration architecture for S3A archives.

---

## 1. Registry Architecture & Descriptors

Historically, knowledge of tile types was scattered across hardcoded pattern matches in `s3a-core` and engine write methods. Phase 2 introduces a formal, centralized `TileTypeRegistry` and descriptor specification (`TileTypeDescriptor`).

### The Descriptor Contract
Every hyper-tile type registered in S3A is described by a `TileTypeDescriptor`:

```rust
pub struct TileTypeDescriptor {
    pub tile_type: u32,
    pub name: &'static str,
    pub owner_pack: &'static str,
    pub required_addon: Option<u32>,
    pub record_size: usize,
    pub current_layout_version: u32,
    pub min_supported_layout_version: u32,
    pub flags: TileTypeFlags,
}
```

### Core & Domain Pack Registry Distribution
- **Core Engine Tiles (`0, 1, 2, 7, 8`)**: Registered automatically via `register_core_types()`. Owned by `"Core Storage Engine"`, `required_addon: None`.
- **Domain Pack Tiles (`3, 4, 5, 6, 10..=15`)**: Domain packs implement `S3AAddon::descriptors(&self)` and export `register_pack_types(&mut TileTypeRegistry)`.
- **Global Registry**: Accessible via `global_registry()` (`OnceLock<RwLock<TileTypeRegistry>>`) and modifiable during startup via `with_global_registry_mut`.

---

## 2. Wire Header Versioning

Each hyper-tile and micro-tile on disk possesses an explicit record layout version in its binary header:

- **Hyper-Tile Header (512 bytes)**: Field `HyperTileHeader.record_layout_version: u32` (offset 44..48, promoted from `_reserved1`).
- **Micro-Tile Header (128 bytes)**: Field `MicroTileHeader.record_layout_version: u16` (offset 34..36, promoted from `_reserved1`).
- **CRC Protection**: `compute_tile_header_crc32` computes CRC32C over `[..crc_offset]` and `[crc_offset + 4..]`, ensuring `record_layout_version` is protected against bit-rot or accidental corruption.

### The Wire Version 0 Alias Rule
Any archive produced before Phase 2 has zeroes in its reserved header bytes:
$$\text{Wire Layout Version } 0 \implies \text{Legacy Compatibility Alias for Version } 1$$
Readers automatically translate layout version `0` to `1` during validation. Writers always stamp `record_layout_version = descriptor.current_layout_version` (typically $\ge 1$).

---

## 3. Schema Evolution Rules: Safe vs. Breaking

S3A distinguishes strictly between additive changes and breaking layout changes:

### Safe / Non-Breaking (In-Place) Changes
1. **Adding New Tile Types**: Assigning a new tile ID in the domain pack's reserved band or starting at $\ge 64$ for third-party packs requires no change to existing tiles.
2. **Utilizing Reserved Pod Padding**: Structs in S3A (such as `AcademicPaperRecord` or `FileHeader`) maintain explicit trailing zero-padding (`_reserved`). Repurposing padding for new optional fields with default zero semantics preserves binary compatibility without bumping layout version.
3. **Payload Extension via Migrators**: Registered `TilePayloadMigrator` hooks can transform payloads lazily during archive upgrade or compaction.

### Breaking Layout Changes
A breaking change occurs when a record layout changes its size, alignment, field order, or field semantics:
1. **Bump Current Version**: `descriptor.current_layout_version` is incremented ($V \to V+1$).
2. **Maintain or Advance Reader Floor**: `descriptor.min_supported_layout_version` defines the oldest layout version this binary knows how to deserialize.
   - If backward-compatible decoding code remains, `min_supported_layout_version` stays at `1`.
   - If an ancient layout is deprecated and dropped, `min_supported_layout_version` is raised to the earliest supported version.
3. **Fail-Closed Gate**:
   - `validate_write`: Any attempt to write a tile with an unsupported or future version is denied with `PermissionDenied`.
   - `validate_read`: If a tile has $\text{version} < \text{min\_supported}$ or $\text{version} > \text{current\_supported}$, reading fails closed with `S3AError::UnsupportedLayoutVersion(v)`.

---

## 4. Compactor Layout Preservation

When the S3A stratified compactor merges or purges archives:
1. **Byte-for-Byte Preservation**: Compactor copy-through of unspecialized and domain pack tiles uses `write_raw_hyper_tile` without modifying `record_layout_version`.
2. **No Silent Resets**: A tile at `record_layout_version = 2` remains at version 2. Legacy tiles at `record_layout_version = 0` remain at version 0 (or are preserved as-is without reinterpretation).
3. **Addon Capability Bitmask Inheritance**: The compacted archive's file header inherits the union of all input archives' `addon_mask` bits.

---

## 5. Scholar Snapshot Version vs. Tile Record Layout Version

In the Scholar Explorer ecosystem, two distinct version fields exist for different abstraction layers:

| Layer | Field | Location | Scope |
|:---|:---|:---|:---|
| **Container Manifest** | `SrsManifestHeader.manifest_version` | Tile 15 | Identifies the container packaging and snapshot layout format (`SCHOLAR_LAYOUT_VERSION_V1 = 1`). Governs snapshot intake/export. |
| **Physical Tile Layout** | `HyperTileHeader.record_layout_version` | Every Hyper-Tile (Tiles 0..=15) | Identifies the binary layout of the records stored within that specific 128 KB tile. Evaluated per-tile by `MmapReader`. |

Both layers fail closed independently, providing layered defense against corrupted or incompatible archives.

---

## 6. Tile ID 9 Collision Resolution

In early prototypes, Tile ID 9 was ambiguously mapped to both an experimental learning activity record and an architectural reservation boundary.

### Resolution Policy
1. **Tile 9 is Strictly `RESERVED_BOUNDARY`**:
   - On the wire and in `TileTypeRegistry`, Tile ID 9 is marked with `TileTypeFlags::RESERVED`.
   - **No user data writes are permitted**: Any attempt to write to Tile 9 via `write_hyper_tile` or `write_raw_hyper_tile` fails closed with `PermissionDenied` (`RegistryError::ReservedTile(9)`).
2. **Greenfield LRS Belonging**:
   - Greenfield Learning Record Store (LRS) and researcher decision ledgers belong exclusively to **Scholar Pack Tile 10** (`HumanLrsRecord`), protected by `ADDON_EDU` (0x0001).
3. **Backward-Compatible Legacy Reading**:
   - Existing prototype archives with Tile 9 data can still be read by `QuerySieve` and S3A-QL queries (`SIFT LEARNING_ACTIVITIES`).
   - `TileType::LEARNING_RECORD_STORE` is marked with `#[deprecated]`.
   - CLI inspection tools (`s3a-cli inspect`) emit a prominent warning when Tile 9 is encountered:
     ```text
     ⚠️  WARNING: Tile type 9 is DEPRECATED/RESERVED_BOUNDARY. Greenfield LRS records should use Scholar Pack Tile 10.
     ```


---

## 7. Copy-Then-Change Rule & Lifecycle States

### Tile Lifecycle States
Every registered tile type possesses an explicit lifecycle state (`TileLifecycle`):
- `Active`: Normal operation; reads and writes permitted.
- `Deprecated`: Read-only. Writes are blocked fail-closed with `RegistryError::DeprecatedTile`. Existing tiles can still be read and compacted.
- `Retired`: Inaccessible. Both reads and writes are rejected fail-closed with `RegistryError::RetiredTile`.

### The Copy-Then-Change Semantic Invariant
When the semantic meaning of an existing field in a record layout changes, **a layout version bump within the same tile type is forbidden**. Instead:
1. **Allocate a new tile type** with the new semantic definition.
2. Mark the previous tile type as `Deprecated` (read-only) or `Retired`.
3. Do not repurpose existing field definitions in place.

#### Known Exception: Tile 12 (Academic Papers)
Tile 12 (`AcademicPaperRecord`) reuses `paper_id: u64` to store integer OpenAlex IDs (`W...` stripped to `u64`). This is recognized as an explicit historical exception grandfathered into V1; no migration is performed at this time. All future semantic shifts must allocate a distinct tile type under the copy-then-change invariant.
