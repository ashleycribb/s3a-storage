# Events Pack

This pack manages events, ticketing, and scheduling.

## Data Model (Layout Offsets)
### EventRecord (280 bytes)
- event_id: UUID @ 0
- thread_id: UUID @ 16
- parent_event_id: UUID @ 32
- sender_pubkey: BYTES @ 48
- recipients_hash: BYTES @ 80
- payload_hash: BYTES @ 112
- prev_event_hash: BYTES @ 144
- created_at_ms: U64 @ 176
- sender_seq: U64 @ 184
- expires_at_ms: U64 @ 192
- kind: U16 @ 200
- flags: U16 @ 202
- payload_len: U32 @ 204
- record_crc32: U32 @ 208
- _pad: U32 @ 212
- signature: BYTES @ 216

### EventRecipientRecord (96 bytes)
- event_id: UUID @ 0
- recipient_pubkey: BYTES @ 16
- grant_id: UUID @ 48
- delivered_at_ms: U64 @ 64
- role: U16 @ 72
- flags: U16 @ 74
- record_crc32: U32 @ 76
- _pad: BYTES @ 80

### LeaseRecord (288 bytes)
- lease_id: UUID @ 0
- holder_pubkey: BYTES @ 16
- resource: STRING @ 48
- grant_id: UUID @ 144
- acquired_at_ms: U64 @ 160
- ttl_ms: U32 @ 168
- action: U16 @ 172
- flags: U16 @ 174
- renew_of: UUID @ 176
- holder_seq: U64 @ 192
- idem_key: UUID @ 200
- record_crc32: U32 @ 216
- _pad: U32 @ 220
- signature: BYTES @ 224

### EventIndexEntry (32 bytes)
- key_hash: U64 @ 0
- event_id: UUID @ 8
- created_at_ms: U64 @ 24

## Registry Proposal
| ID | Name | Pack | Lifecycle | Size |
|---|---|---|---|---|
| 112 | event_record | s3a-events | Active | 280 |
| 113 | event_recipient_record | s3a-events | Active | 96 |
| 114 | lease_record | s3a-events | Active | 288 |
| 115 | event_index_derived | s3a-events | Active | 32 |

## Facts
Verified free tile ids: 112..=119
Verified `DelegationGrantRecord` can hold role labels in `allowed_scopes`
Verified `ApprovalCheckpointRecord` can hold `resolution_ref`
Verified `AgentPass` can hold `events.lease` with `slot://` resources
Verified BLOB pattern holds large objects
Verified `Thing` can sign
Verified roles fit in `allowed_scopes`

## Integration in Reader
To show these layouts in `s3a-reader`, append:
```rust
for l in s3a_events::display_layouts() { hyper.register(l) }
```
