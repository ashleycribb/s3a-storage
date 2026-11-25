# s3a-spatial design principles

Source: the nine lessons from 3D file-standards construction, recorded in
`~/workspace/s3a-spatial/research/file-standards-for-s3a.md` §14 ("Lessons for a
250-year engine"), Nov 19, 2026. The lessons come from how glTF, USD, 3D Tiles,
VRM, CityGML/IFC, X3D, and Gaussian splats are constructed — manifest+payload,
containers-within-containers — and from what makes a format readable 250 years
after it was written.

These rules govern every step of this pack (tiles 33–39 and beyond). A step that
breaks one needs an explicit, recorded exception in its test log — never a silent
one. Each principle names its status in the pack today, so a future reader can
tell what is proven from what is still a rule to be kept.

## 1. Manifest first

A GLB's JSON chunk describes everything in the file before a byte of geometry is
read. Every archive and tile leads with a readable manifest — registry, schema,
index — so a future reader understands what's inside before reading a single
record.

**Status: pack rule.** Tile 33's registry descriptor and fixed record layout are
the manifest for its records. Tile 38's per-type descriptor JSONs carry the
manifest for asset references (see the standards doc §9).

## 2. Description and data live apart

glTF's JSON vs BIN; USD's layers vs payloads. Description is small, stable, and
cheap to copy; data is big and changes.

**Status: implemented (pack-wide).** S3A's metadata-only rule, kept strict: this
pack stores metadata and hashes, never asset bytes. A record describes; the
payload lives outside under a hash.

## 3. Hash what you received, not what you think it means

**Status: implemented in step 1.** The `content_hash` on `GeoposeBasicQuatRecord`
is SHA-256 of the original file bytes, not of a re-serialization. Trust attaches
to the file as received; any re-serialization gets its own hash.

## 4. Compose by reference, never by flattening

USD layers, 3D Tiles external tilesets, glTF 2.1 External Assets — big things
built by pointing at small things, non-destructively.

**Status: implemented in step 1 (pattern).** `origin_ptr` on the tile-33 record;
`geopose_ptr` planned for tile 38, linking an asset reference to its tile-33
record. Never copy-and-modify the source, and never bake the resolved view into
storage — the Navigator resolves at view time.

## 5. Version everything, guess nothing

VRM 0.x faces −Z and 1.0 faces +Z, and nothing in the file says which — the
version must be declared and read.

**Status: implemented.** Every record carries its version; readers fail closed on
unknown versions instead of guessing. The cautionary example is the s3a-storage
#1 schema fix: tour chose a decoder from schema shape instead of the tile's
declared `record_layout_version`.

## 6. Deprecate openly; a conversion is provenance

3D Tiles deprecated its old containers rather than silently replacing them, and
converting IFC to CityGML loses meaning.

**Status: pack rule for future steps.** Tombstone, don't delete — a new sealed
version with the old one marked, and the conversion itself recorded as a
provenance event.

## 7. Rights travel with the data

VRM carries its license metadata inside the file; the formats that get adopted in
the open carry their license with them.

**Status: APPROVED IN PRINCIPLE (Ashley, Nov 19, 2026) — details deferred to
tile 38.** Recording license/rights metadata is fine as long as it doesn't impede
on people's rights: the field describes the rights that came with the asset, it
never adds restrictions of S3A's own, and it is never used as an access gate —
it must not withhold someone's own data from them. VRM's in-file `meta` is the
model, and tile 38's descriptor is where it lands. Do NOT add it to tile 33: the
sealed 128-byte layout is already pushed, and changing it would break the sealed
record.

## 8. Keep the container simple enough to parse from the spec alone

A GLB is a 12-byte header plus length-prefixed chunks; anyone can write a parser
in an afternoon. The USD crate needs a real implementation. The 250-year test:
could someone with only the spec and the bytes read it, verify it, and trust it?

**Status: pack rule.** Fixed headers, explicit lengths, no clever compression
only one library understands. Step 1's hand-written JSON number reader is the
example of the rule in action: serde_json mis-parsed one OGC value by 1 ULP, so
the pack refused a dependency that hides behaviour and reads numbers with
Rust's correctly-rounded `str::parse::<f64>()` instead.

## 9. A spec survives through its tests

Khronos has conformance tests and sample models for a reason. A future
implementer proves they read the format right by passing the tests.

**Status: implemented.** Test-first, always: AT-SP-01..AT-SP-05 for step 1, and
the same shape for every later step. Tests are never edited to pass. Each step's
test log names which principles it exercised.
