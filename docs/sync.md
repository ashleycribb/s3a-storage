# S3A Explorer to LRS Batched Synchronization Contract

This specification defines the client-to-server synchronization contract for Scholar Explorer, AI agents in Google AI Studio, and the S3A Learning Record Store (LRS) server.

---

## 1. Architectural Overview

Scholar Explorer operates as an offline-first WebAssembly (WASM) additive research notebook. All research activities—human reading decisions, note and claim extractions, paper bookmarks, and AI reasoning traces—are appended immediately to an in-memory `WasmAgentSnapshot` with zero cloud latency.

To synchronize this research provenance with the centralized VM LRS (`scholar.s3a`), the client executes **batched, cryptographically signed, append-only delta synchronizations**.

```
+-------------------------------------------------------------------------+
|                       Scholar Explorer (WASM / Browser)                 |
|                                                                         |
|  [User Actions / AI Steps] ---> [WasmAgentSnapshot.statement_log]       |
|                                            |                            |
|                                 export_since(local_cursor)              |
|                                            |                            |
|                                    S3ASYNC1 Packet                      |
|                                            |                            |
|                              Ed25519 Sign with Session Key              |
+--------------------------------------------|----------------------------+
                                             |
                  POST /xapi/s3a/batch       | (HTTP Basic Auth + TLS)
                  Payload: { session_id, cursor_range, bytes, checksum, signature }
                                             |
+--------------------------------------------v----------------------------+
|                         S3A VM LRS Server                               |
|                                                                         |
|  1. Verify Basic Auth & Scope (statements/write)                        |
|  2. Enforce Body Size (<= 5 MB) & Session Rate Limit (<= 60/min)        |
|  3. Validate Hardware CRC32C Checksum                                   |
|  4. Validate Ed25519 Signature against Public Key                       |
|  5. Check Cursor Progression:                                           |
|       - Exact duplicate [from, to] == [last_from, last_to] -> 200 No-op |
|       - Gap (from_cursor > last_cursor)                   -> 409 Conflict
|       - Out-of-order (from_cursor < last_cursor)          -> 409 Conflict
|       - In-order (from_cursor == last_cursor)             -> Proceed    |
|  6. Append Native Hyper-Tiles directly to scholar.s3a                   |
|  7. Return 200 OK: { accepted_cursor, duplicate: false, ... }           |
+-------------------------------------------------------------------------+
```

---

## 2. Synchronization Triggers

The client must trigger synchronization upon any of the following three events:

1. **Scheduled Periodic Sync:**
   - Every **6 hours** by default (`21,600,000 ms`).
   - Implemented via `window.setInterval` or background timer.
2. **Session Lifecycle (Logout / Window Close):**
   - Upon explicit user logout (`record_human(VERB_ID_LOGGED_OUT, ...)`).
   - Upon window / tab teardown via `beforeunload` or `pagehide` using `navigator.sendBeacon` or blocking fetch.
3. **Statement Threshold Trigger:**
   - Whenever the count of un-synced statements since `local_cursor` exceeds **500 statements** (`pending_delta_count >= 500`).

---

## 3. Strict Consent Gating

Privacy and consent are enforced fail-closed at the client layer before any network traffic is emitted:

- **Affirmative Consent Required:**
  - Synchronization MUST only run when the user has explicitly consented to cloud telemetry / research synchronization (`consent_granted === true`).
- **Consent Revocation / Opt-Out:**
  - If the user declines consent or revokes prior consent (recorded locally via verb `consent-declined`, ID 21), synchronization is immediately disabled.
  - The client transmits **zero network requests** (`0` bytes sent over the network).
  - Un-synced statements remain stored strictly within the user's local browser storage.

---

## 4. Cursor Progression & Idempotency Protocol

To guarantee zero data loss and prevent duplicate statement ingestion on unstable network connections:

1. **Local Cursor State:**
   - The client persists `local_cursor: u64` in `IndexedDB` or `localStorage`. Initial value is `0`.
2. **Delta Generation:**
   - Client calls `export_since(local_cursor)`.
   - Returns delta bytes (`bytes`), new cursor (`new_cursor`), and `record_count`.
   - If `record_count === 0`, no HTTP request is necessary unless confirming session liveness.
3. **Transmission:**
   - Client sends `POST /xapi/s3a/batch` with `from_cursor = local_cursor` and `to_cursor = new_cursor`.
4. **Advance Rule:**
   - The client **MUST NOT** advance `local_cursor` until the server returns HTTP `200 OK` confirming `accepted_cursor`.
   - Upon receiving HTTP 200 OK, `local_cursor = response.accepted_cursor`.

### Idempotency Guarantee
If a client experiences network disconnection before receiving the server's HTTP 200 response, it may safely re-transmit the exact same batch.
- When the server detects `from_cursor == last_from && to_cursor == last_to`, it performs a **200 OK no-op**.
- The server does not append duplicate records to `scholar.s3a` and returns `{ duplicate: true, accepted_cursor: ... }`.

---

## 5. API Endpoint Specification

### `POST /xapi/s3a/batch` (or `POST /s3a/batch`)

#### Headers
- `Authorization: Basic <base64(user:password)>` (LRS credentials with `statements/write` or `all` scope).
- `Content-Type: application/json`

#### Request Schema
```json
{
  "session_id": "0123456789abcdef0123456789abcdef",
  "from_cursor": 0,
  "to_cursor": 5,
  "cursor_range": [0, 5],
  "bytes": "<base64 encoded S3ASYNC1 packet>",
  "checksum": 2849182391,
  "signature": "<64-byte Ed25519 signature in hex or base64>",
  "public_key": "<32-byte Ed25519 public key in hex or base64>"
}
```

#### Response Schemas

**Success (200 OK):**
```json
{
  "status": "ok",
  "session_id": "0123456789abcdef0123456789abcdef",
  "accepted_cursor": 5,
  "duplicate": false,
  "records_appended": 5
}
```

**Duplicate Re-send (200 OK):**
```json
{
  "status": "ok",
  "session_id": "0123456789abcdef0123456789abcdef",
  "accepted_cursor": 5,
  "duplicate": true,
  "records_appended": 0
}
```

**Gap / Out of Order Detected (409 Conflict):**
```json
{
  "error": "Gap detected: session expected from_cursor 5, but received 10"
}
```

**Payload Too Large (413 Payload Too Large):**
```json
{
  "error": "Payload Too Large: batch size 6291456 bytes exceeds maximum allowed 5242880 bytes"
}
```

**Rate Limit Exceeded (429 Too Many Requests):**
```json
{
  "error": "Rate limit exceeded: maximum 60 batch requests per minute for session"
}
```

---

## 6. Fault Tolerance & Retry Backoff

When network failures, 5xx server errors, or 429 rate limit responses occur, clients must apply exponential backoff with jitter:

$$T_{\text{wait}} = \min(T_{\max}, T_{\text{base}} \times 2^{\text{attempt}}) \pm \text{jitter}$$

- **Base interval:** 1,000 ms
- **Max interval:** 60,000 ms
- **Max retries per sync attempt:** 5
- If max retries are exceeded, the client retains the un-synced statements locally and defers to the next scheduled 6-hour interval.

---

## 7. Server Operating Limits

| Parameter | Default Value | CLI Flag | Description |
|---|---|---|---|
| Max Batch Body Size | 5 MB (`5,242,880 bytes`) | `--max-batch-size` | Reject oversized payloads with HTTP 413 |
| Max Session Rate | 60 requests / minute | `--max-batch-rate` | Prevent runaway sync loops with HTTP 429 |
| Storage Archive | `scholar.s3a` | Assigned via creds or data directory | Target archive receiving synced tiles |
