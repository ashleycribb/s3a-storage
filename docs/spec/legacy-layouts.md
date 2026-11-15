# Legacy record layouts (read-only)

Readers still decode these layout v1 tiles; writers no longer produce them. Values a v1 record
does not hold (paper title, venue name, DOI, authors; edge actor) are absent when read, never
invented. A zero `timestamp_sec` means unknown and is absent.

## Tile 12 `academic_papers`, layout v1 (64-byte records)

| Field ID | Name | Field Type | Byte Offset | Byte Length | Description |
|:---:|:---|:---:|:---:|:---:|:---|
| 1 | `paper_id` | U64 (4) | 0 | 8 | DOI / URI hash |
| 2 | `timestamp_sec` | U64 (4) | 8 | 8 | Publication timestamp |
| 3 | `hilbert_index` | U64 (4) | 16 | 8 | 3D Hilbert curve index |
| 4 | `topic_x` | F32 (9) | 24 | 4 | Topic cluster coordinate X |
| 5 | `topic_y` | F32 (9) | 28 | 4 | Topic cluster coordinate Y |
| 6 | `topic_z` | F32 (9) | 32 | 4 | Topic cluster coordinate Z |
| 7 | `citation_count` | U32 (3) | 36 | 4 | Citation count |
| 8 | `year` | U16 (2) | 40 | 2 | Publication year |
| 9 | `venue_id` | U16 (2) | 42 | 2 | Publication venue ID (a code, not a name) |
| 10 | `open_access_flag` | U16 (2) | 44 | 2 | 1 = open access |
| 11 | `warning_count` | U16 (2) | 46 | 2 | Provenance warnings count |
| 12 | `doi_prefix_hash` | U64 (4) | 48 | 8 | Publisher DOI prefix hash |
| 13 | `flags` | U64 (4) | 56 | 8 | Status flags |

## Tile 13 `research_graph`, layout v1 (64-byte records)

| Field ID | Name | Field Type | Byte Offset | Byte Length | Description |
|:---:|:---|:---:|:---:|:---:|:---|
| 1 | `subject_hash` | U64 (4) | 0 | 8 | Subject node identifier hash |
| 2 | `object_hash` | U64 (4) | 8 | 8 | Object node identifier hash |
| 3 | `timestamp_sec` | U64 (4) | 16 | 8 | Assertion timestamp in epoch seconds (0 = unknown) |
| 4 | `hilbert_coord` | U64 (4) | 24 | 8 | Spatial coordinate |
| 5 | `predicate_id` | U32 (3) | 32 | 4 | Predicate relation code |
| 6 | `weight` | F32 (9) | 36 | 4 | Edge weight or confidence |
| 7 | `flags` | U64 (4) | 40 | 8 | Status flags |
