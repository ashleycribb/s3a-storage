# S3A Digital Vault Specification (S3A-VAULT)

**Document Version:** 1.0-draft  
**Status:** IMPLEMENTED (Phase 1)  
**Date:** October 2026  
**License:** [Creative Commons Attribution 4.0 International (CC-BY-4.0)](LICENSE)  
**Patents:** [Royalty-Free Patent Non-Assertion Pledge](PATENTS.md)  
**Parent Specification:** [S3A Hyper-Tile Storage Format Specification](S3A-TILE-SPEC.md)  
**Companion Specification:** [S3A Edge Profile Specification](S3A-EDGE-PROFILE.md)

---

## Specification Audit & Implementation Notice

> [!IMPORTANT]
> **SPECIFICATION AUDIT STATUS: IMPLEMENTED (Phase 1)**  
> In accordance with the S3A Specification Audit Policy, the core Phase 1 architecture of the S3A Digital Vault has been **fully implemented and verified**. This implementation includes sovereign identity custody (`crates/s3a-vault`), encrypted key storage (`FileKeyStore`), gate network addressing (`GateAddress`), bounded delegation grants with non-delegable judgment protection (`s3a-harness`), access code sessions, transfer ownership lineage, key rotation and revocation, encrypted backup/restore, the Gate Directory and relay mailbox service in `s3a-lrs-server` (`--gate-directory`), the `s3a-vault` CLI binary, and normative conformance vectors `VAULT-VEC-01` through `VAULT-VEC-04` in `crates/s3a-spec-vectors`. All 303 tests across the repository workspace pass (`cargo test --workspace`).

---

## 1. Conformance, Principles & Explicit Scope

### 1.1 Conformance Terminology

The key words **"MUST"**, **"MUST NOT"**, **"REQUIRED"**, **"SHALL"**, **"SHALL NOT"**, **"SHOULD"**, **"SHOULD NOT"**, **"RECOMMENDED"**, **"NOT RECOMMENDED"**, **"MAY"**, and **"OPTIONAL"** in this document are to be interpreted as described in BCP 14 [RFC 2119] [RFC 8174].

### 1.2 Architectural Foundation: A Vault is an S3A Archive

> **PRIMARY ARCHITECTURAL PRINCIPLE**:  
> An S3A Vault **IS A VALID S3A ARCHIVE FILE** (`.vault.s3a` or `.s3a`).  
> A vault file adheres strictly to the S3A DualFileHeader framing (§2 of `S3A-TILE-SPEC.md`) or the Edge Profile framing (`S3A-EDGE-PROFILE.md`). Any compliant S3A reader, inspector, or compactor can open, inspect, and verify an S3A Vault without specialized proprietary software.

### 1.3 Explicit Non-Financial Scope Boundary

> [!WARNING]
> **STRICT OUT-OF-SCOPE DECLARATION**:  
> The S3A Vault is an architecture for **cryptographic identity, physical asset custody, autonomous agent delegation, educational credentials (W3C VC / Open Badges 3.0), research provenance, gate network addressing, approval checkpoints, and non-fungible digital holdings**.  
> **Money, fiat currencies, payments, banking transactions, cryptocurrencies, payment tokens, and decentralized finance (DeFi) primitives are STRICTLY OUT OF SCOPE.**

---

## 2. Identities & Key Management

An S3A Vault anchors cryptographically verifiable identities for natural persons, autonomous AI agents, organizations, and physical edge things.

### 2.1 Owner Kinds

An identity in the vault MUST declare one of four canonical owner kinds:

```rust
#[repr(u16)]
pub enum VaultOwnerKind {
    Person = 1,       // Natural human subject / learner / researcher
    Agent = 2,        // Autonomous software / AI agent executing via harness
    Organization = 3, // Academic institution, laboratory, IRB, or enterprise
    Thing = 4,        // Embedded sensors, edge gateways, lab instruments, equipment
}
```

### 2.2 Cryptographic Key Separation & Storage Boundaries

All identity signatures and attestations within the vault use standard **Ed25519** (Edwards-curve Digital Signature Algorithm, RFC 8032):
- **Public Key**: Exactly 32 bytes (`[u8; 32]`).
- **Signature**: Exactly 64 bytes (`[u8; 64]`).

To eliminate key exfiltration vulnerabilities, S3A establishes strict physical and architectural isolation boundaries:

```
+-----------------------------------------------------------------------+
| S3A Cryptographic Key Storage Isolation Boundaries                    |
+-----------------------------------------------------------------------+
| PERSON IDENTITY KEYS:                                                 |
| - Stored exclusively in Hardware Secure Enclaves / Keystores.        |
| - Android: Android Keystore (`MasterKeyProvider`, Hardware StrongBox) |
| - iOS / macOS: Secure Enclave Processor (SEP)                         |
| - Linux / Windows: TPM 2.0 / PKCS#11 Hardware Security Token          |
| - Private key material NEVER touches user-space RAM unencrypted.     |
+-----------------------------------------------------------------------+
| AGENT IDENTITY KEYS:                                                  |
| - Generated and retained exclusively by the runtime harness (e.g.     |
|   `s3a-harness` or sandboxed agent executor).                         |
| - Model Isolation Rule: Private keys are NEVER exposed to the AI      |
|   language model, context window, prompts, or completion buffers.     |
| - Model requests action -> Harness validates grant -> Harness signs.  |
+-----------------------------------------------------------------------+
| ORGANIZATION IDENTITY KEYS:                                           |
| - Multi-signature quorum or FIPS 140-2 Level 3 Hardware Security      |
|   Module (HSM) / Cloud KMS with audit logging.                        |
+-----------------------------------------------------------------------+
| THING IDENTITY KEYS:                                                  |
| - Hardware-anchored crypto elements: Microchip ATECC608A / Optiga    |
|   Trust M, embedded Secure Element (eSE), or firmware TPM / PUF.      |
| - Private key sealed to device firmware integrity; non-extractable.   |
| - Edge runtime signs telemetry & seals micro-tiles on device.         |
+-----------------------------------------------------------------------+
```

### 2.3 VaultIdentityRecord Binary Layout

Identity records are plain-old-data (Pod) structs embedded within an Identity/Record tile:

#### Layout (96 Bytes, 8-Byte Aligned, Little-Endian)
| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `identity_id` | Unique 128-bit UUID identifying this entity. |
| `16 .. 18` | `u16` | `owner_kind` | `1 = Person`, `2 = Agent`, `3 = Organization`, `4 = Thing`. |
| `18 .. 20` | `u16` | `key_algorithm` | MUST be `1` (`ALG_ED25519`). |
| `20 .. 52` | `[u8; 32]` | `public_key` | 32-byte Ed25519 raw public key. |
| `52 .. 84` | `[u8; 32]` | `display_name` | Null-terminated UTF-8 human-readable label. |
| `84 .. 92` | `u64` | `created_at` | Unix epoch timestamp of identity creation. |
| `92 .. 96` | `u32` | `record_crc32` | Castagnoli CRC32C checksum over bytes `0..92`. |

---

## 3. Delegation Grants & Agent Governance

Autonomous AI agents act as representatives of human users, organizations, or supervised physical installations. To prevent unauthorized or rogue operations, all agent capabilities require explicit, cryptographically signed, bounded **Delegation Grants**.

### 3.1 Delegation Grant Structure

A principal issues a grant by signing a `DelegationGrantRecord`:

```
+---------------------------------------------------------------+
| DelegationGrantRecord (192 Bytes, 8-Byte Aligned)             |
|   grant_id: [u8; 16] (128-bit UUID)                           |
|   granter_pubkey: [u8; 32] (Ed25519 Public Key of Principal)  |
|   agent_id: [u8; 16] (UUID of Authorized Agent)               |
|   agent_pubkey: [u8; 32] (Ed25519 Public Key of Agent)        |
|   allowed_actions: u32 (Permitted Capabilities Bitmask)       |
|   allowed_scopes: u32 (Allowed Tile Types / Record Mask)      |
|   max_operations: u32 (Maximum Allowed Action Invocations)    |
|   issued_at: u64 (Unix Epoch Timestamp)                       |
|   expires_at: u64 (Hard Expiration Timestamp)                 |
|   granter_signature: [u8; 64] (Ed25519 Signature by Granter)  |
+---------------------------------------------------------------+
```

#### 3.1.1 Allowed Actions Bitmask (`allowed_actions`)
- `0x0001` — `ACTION_READ`: Query and read holdings and records within allowed scopes.
- `0x0002` — `ACTION_WRITE`: Append new telemetry, notes, or analytical results.
- `0x0004` — `ACTION_SEND_ITEM`: Transfer a holding or credential to an external recipient.
- `0x0008` — `ACTION_ISSUE_CREDENTIAL`: Co-sign or assert an educational/skill competency.
- `0x0010` — `ACTION_ASK_HUMAN`: Route request to human operator for interactive approval.

### 3.2 Anti-Escalation & Governance Rules

The vault engine strictly enforces the following immutable governance invariants:

1. **No Self-Granting**: Agents MUST NOT author, sign, or commit delegation grants to themselves. Grants signed where `granter_pubkey == agent_pubkey` with `owner_kind == Agent` are invalid.
2. **No Widening**: An agent operating under a grant MUST NOT sub-delegate authorities wider than its active grant (`sub_grant.allowed_actions & !active_grant.allowed_actions == 0`).
3. **Mandatory Grant Provenance**: Every write, transfer, or assertion executed by an agent MUST record the `grant_id` relied upon.
4. **Limits & Expiry**: The vault engine tracks operations executed under each grant. Once `invocations >= max_operations` or current time exceeds `expires_at`, further agent requests are rejected (`GrantExpiredOrExhausted`).
5. **Non-Delegable Judgment Actions**:
   > **THE JUDGMENT INVARIANT**:  
   > High-consequence decisions can NEVER be delegated to autonomous models.  
   > Subjective claim assessments (claim support, methodological rigor, overstatement, extrapolation), irreversible record destruction (`erasure`), consent withdrawal (`consent`), physical actuator setpoint overrides, legal waiver signing, and delegation revocation CANNOT be executed autonomously by agents.  
   > When an agent encounters a judgment action, it MUST invoke `ACTION_ASK_HUMAN` to request explicit, interactive human biometric or hardware authorization.

---

## 4. Holdings & Verifiable Credentials

An S3A Vault manages digital holdings, including learning achievements, licenses, intellectual assets, research artifacts, and W3C Verifiable Credentials.

### 4.1 VaultItemRecord Binary Layout

Each holding is registered as an item record:

#### Layout (160 Bytes, 8-Byte Aligned, Little-Endian)
| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `item_id` | Unique 128-bit UUID of the holding. |
| `16 .. 18` | `u16` | `item_type` | Holding category (§4.2). |
| `18 .. 20` | `u16` | `item_flags` | Flags: `0x01 = Unique`, `0x02 = VerifiableCredential`, `0x04 = Encrypted`. |
| `20 .. 52` | `[u8; 32]` | `item_hash` | SHA-256 cryptographic digest of the holding payload. |
| `52 .. 84` | `[u8; 32]` | `issuer_pubkey` | 32-byte Ed25519 public key of the issuing authority. |
| `84 .. 116`| `[u8; 32]` | `current_owner_pubkey`| 32-byte Ed25519 public key of the current owner. |
| `116 .. 124`| `u64` | `acquired_at` | Unix epoch seconds when ownership was registered. |
| `124 .. 128`| `u32` | `consent_scope` | Participant privacy and redistribution mask (§8). |
| `128 .. 144`| `[u8; 16]` | `payload_ref` | Coordinate or Blob UUID referencing raw payload data. |
| `144 .. 156`| `[u8; 12]` | `_reserved` | Explicit zero padding. |
| `156 .. 160`| `u32` | `record_crc32` | Castagnoli CRC32C over bytes `0..156`. |

### 4.2 Canonical Item Categories (`item_type`)
- `0x0001` — `ITEM_VERIFIABLE_CREDENTIAL`: W3C VC or Open Badges 3.0 credential.
- `0x0002` — `ITEM_LEARNER_RECORD`: IEEE P2997 Enterprise Learner Record assertion.
- `0x0003` — `ITEM_RESEARCH_DATASET`: Sealed research dataset artifact or snapshot.
- `0x0004` — `ITEM_ALGORITHMIC_MODEL`: Model weights, fine-tune adapters, or prompt recipes.
- `0x0005` — `ITEM_LICENSE_GRANT`: Digital rights, usage permissions, or data access tokens.
- `0x0006` — `ITEM_BINARY_ATTACHMENT`: Raw PDF, media, or arbitrary scientific asset.

### 4.3 Verifiable Credentials & Open Badges 3.0 Mapping

S3A Vaults provide native, bidirectional mapping for **W3C Verifiable Credentials (VC v2.0)** and **1EdTech Open Badges 3.0**:

```mermaid
flowchart LR
    A["W3C Verifiable Credential / Open Badges 3.0 (JSON-LD)"] <--> B["S3A Translation Layer"]
    B --> C["Core Catalog: Chunked Blob Tile (68) Raw JSON-LD"]
    B --> D["Core ELR: elr_credential_assertion Tile (30)"]
    B --> E["Vault: VaultItemRecord Index (Type 1)"]
```

1. **Storage Format**:
   - The original JSON-LD credential document is stored verbatim in a Chunked Blob Tile (`blob`, Type ID: `68` / `0x0044`), preserving byte-level integrity for canonical signature verification.
   - Fast query attributes (learner ID, competency URI, achievement title, issuer DID, issue date) are indexed in Core ELR tiles (`elr_credential_assertion`, Type ID: `30` / `0x001E`).
   - A `VaultItemRecord` is committed to represent custody in the vault.
2. **Signature Verification**:
   - The vault engine validates the credential's cryptographic proof suite (`DataIntegrityProof` with `ed25519-2020` cryptosuite or `Ed25519Signature2020`).
   - The issuer DID (e.g. `did:key:z6Mku...`) is resolved to an Ed25519 public key and verified against `issuer_pubkey`.

---

## 5. Transfers & Ownership Chains

Holdings in an S3A Vault can be transferred between vaults or copied with provenance.

### 5.1 ItemTransferRecord Binary Layout

Ownership transitions are recorded as signed transfer links:

#### Layout (176 Bytes + 64 Byte Signature = 240 Bytes, 8-Byte Aligned, Little-Endian)
| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `transfer_id` | Unique 128-bit UUID of the transfer transaction. |
| `16 .. 48` | `[u8; 32]` | `item_hash` | SHA-256 digest of the holding being transferred. |
| `48 .. 80` | `[u8; 32]` | `from_pubkey` | 32-byte Ed25519 public key of the transferring owner. |
| `80 .. 112`| `[u8; 32]` | `to_pubkey` | 32-byte Ed25519 public key of the recipient. |
| `112 .. 120`| `u64` | `timestamp` | Unix epoch timestamp of transfer execution. |
| `120 .. 152`| `[u8; 32]` | `prev_transfer_hash`| SHA-256 digest of prior transfer in the ownership chain. |
| `152 .. 154`| `u16` | `transfer_mode`| `0 = UniqueTransfer`, `1 = CopyWithProvenance` (§5.2). |
| `154 .. 160`| `[u8; 6]` | `_pad` | Alignment padding. |
| `160 .. 172`| `[u8; 12]` | `_reserved` | Reserved zero padding. |
| `172 .. 176`| `u32` | `record_crc32` | Castagnoli CRC32C over bytes `0..172`. |
| `176 .. 240`| `[u8; 64]` | `sender_signature` | Ed25519 signature by `from_pubkey` over bytes `0..176`. |

### 5.2 Transfer Modes

1. **`UniqueTransfer` (`0`)**:
   - Transfers exclusive ownership of a unique holding (e.g., an individual non-fungible certificate or unique license).
   - Once executed, the sender's vault marks the item inactive/tombstoned.
   - The recipient vault appends the transfer link to establish current ownership.
2. **`CopyWithProvenance` (`1`)**:
   - Used for non-exclusive research datasets, educational resources, and badges.
   - Duplicates the holding to the recipient vault while preserving full upstream lineage (`prev_transfer_hash` and `issuer_pubkey`). Both sender and recipient hold valid attested copies.

### 5.3 Ownership Resolution & Double-Send Prevention

$$\text{transfer\_hash}_n = \text{SHA-256}(\text{prev\_transfer\_hash} \parallel \text{item\_hash} \parallel \text{from\_pubkey} \parallel \text{to\_pubkey} \parallel \text{timestamp})$$

1. **Chain Verification**: A verifier traces the transfer chain backward to genesis (`prev_transfer_hash == [0u8; 32]`, signed by `issuer_pubkey`).
2. **Current Owner Rule**: The authoritative current owner of a unique holding is the `to_pubkey` of the highest-sequence validly signed transfer link.
3. **Double-Send Prevention**:
   - **Local Ledger**: The vault engine rejects any transfer where `from_pubkey` has already signed a subsequent transfer for that `item_hash`.
   - **Networked / LRS Registry Check**: For high-assurance institutional transfers, vaults perform an optional consensus lookup against the central S3A LRS server registry (`s3a-lrs-server` state commitment check) to assert no conflicting transfer has been recorded.

---

## 6. Revocation, Key Rotation & Disaster Recovery

### 6.1 Key Rotation (`KeyRotationRecord`)

When an owner upgrades hardware, migrates to a new device, or discovers a compromised key, a `KeyRotationRecord` is committed:

#### Layout (220 Bytes, 8-Byte Aligned, Little-Endian)
| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `rotation_id` | Unique 128-bit UUID. |
| `16 .. 48` | `[u8; 32]` | `old_public_key` | Public key being retired. |
| `48 .. 80` | `[u8; 32]` | `new_public_key` | Newly authorized replacement public key. |
| `80 .. 88` | `u64` | `rotated_at` | Unix epoch timestamp of rotation. |
| `88 .. 152`| `[u8; 64]` | `superseded_sig` | Signature by `old_public_key` authorizing transition. |
| `152 .. 216`| `[u8; 64]`| `confirm_sig` | Signature by `new_public_key` confirming receipt. |
| `216 .. 220`| `u32` | `record_crc32` | Castagnoli CRC32C over bytes `0..216`. |

### 6.2 Revoked-Key Enforcement

The vault maintains an append-only `RevokedKeyRecord` table. Once a key appears in the revocation registry:
1. All future actions, transfers, or grants signed by the revoked key are rejected (`KeyRevokedError`).
2. Prior signatures committed before `rotated_at` remain valid, preserving historical auditability.

### 6.3 Disaster Recovery Mechanisms

1. **Second Device Pairing (Guardian Authorization)**:
   - The user provisionally enrolls a secondary hardware device (e.g. tablet or security key) with an independent keypair.
   - In the event of primary device loss, the secondary device can co-sign a `KeyRotationRecord` alongside an emergency recovery grant.
2. **Deterministic Recovery Phrase**:
   - Standard 12- or 24-word BIP-39 mnemonic seed phrase mapping deterministically via SLIP-0010 to an Ed25519 master identity key (`m/44'/607'/0'/0'/0'`).
3. **Encrypted Vault Backup Container**:
   - Full vault backup exported as an encrypted `.snapshot.s3a` container:
     - Encrypted with **XChaCha20-Poly1305** using a 256-bit symmetric key derived via **Argon2id** (memory: 64 MB, iterations: 3, parallelism: 4) from the user's master passphrase.
     - Protected with Reed-Solomon erasure parity ($K=16, M=2$).
     - Anchored to OpenTimestamps and RFC 3161 calendars.

---

## 7. Append-Only Event History & Cryptographic Sealing

Every action within the vault is permanently recorded in an append-only event log tile.

### 7.1 VaultEventRecord Binary Layout (80 Bytes, 8-Byte Aligned)

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 8` | `u64` | `event_sequence`| Monotonically increasing event sequence counter ($1, 2, 3, \dots$). |
| `8 .. 16` | `u64` | `timestamp` | Unix epoch timestamp of event execution. |
| `16 .. 18` | `u16` | `event_type` | Event category (§7.2). |
| `18 .. 20` | `u16` | `actor_kind` | `1 = Person`, `2 = Agent`, `3 = Organization`, `4 = Thing`. |
| `20 .. 36` | `[u8; 16]` | `target_id` | UUID of affected item, grant, key, or checkpoint. |
| `36 .. 68` | `[u8; 32]` | `prev_event_hash`| SHA-256 digest of preceding event record. |
| `68 .. 76` | `u64` | `grant_id_suffix`| Lower 64 bits of `grant_id` relied upon (0 if human/thing). |
| `76 .. 80` | `u32` | `record_crc32` | Castagnoli CRC32C over bytes `0..76`. |

### 7.2 Canonical Vault Event Types (`event_type`)
- `0x0001` — `EVENT_ITEM_RECEIVED`: Inbound transfer or credential import.
- `0x0002` — `EVENT_ITEM_SENT`: Outbound transfer executed.
- `0x0003` — `EVENT_GRANT_ISSUED`: Delegation grant signed for an agent.
- `0x0004` — `EVENT_GRANT_REVOKED`: Delegation grant cancelled early.
- `0x0005` — `EVENT_GRANT_USED`: Agent performed action under active grant.
- `0x0006` — `EVENT_KEY_ROTATED`: Identity keypair rotated.
- `0x0007` — `EVENT_CONSENT_MODIFIED`: Consent scope updated for holding.
- `0x0008` — `EVENT_CHECKPOINT_EVALUATED`: Approval checkpoint evaluated (approved/rejected/escalated).
- `0x0009` — `EVENT_THING_PROVISIONED`: Physical asset enrolled into vault custody.

### 7.3 Container Sealing Integration

Periodically or upon export, the entire vault archive is sealed using S3A Sealed Container v2 framing (§10 of `S3A-TILE-SPEC.md`):
- `SealedIndexHeaderV2` records `seal_generation` and links `prev_seal_hash`.
- `seal_hash` commits to all vault tiles and the complete event history.
- The sealed container endcap (`b"S3AEND\0\x02"`) makes the vault ledger tamper-evident.

---

## 8. Consent Governance & Export Enforcement

Holdings in an S3A Vault carry participant consent metadata honoring GDPR, HIPAA, and IRB research participant rights directly at the storage boundary.

### 8.1 Consent Scope Bitmask (`consent_scope`)

Every `VaultItemRecord` embeds a 32-bit `consent_scope` bitmask:
- `0x0001` — `SCOPE_RESEARCH_ONLY`: Restricted strictly to academic/scientific research; commercial analytics forbidden.
- `0x0002` — `SCOPE_INTERNAL_ENTERPRISE`: Permitted only within the owning organization.
- `0x0004` — `SCOPE_NO_REEXPORT`: Recipient cannot re-transfer or export outside their vault.
- `0x0008` — `SCOPE_HUMAN_APPROVAL_ON_EXPORT`: Autonomous agents CANNOT export this item; requires explicit human biometric consent.
- `0x0010` — `SCOPE_MODEL_TRAINING_FORBIDDEN`: Data cannot be ingested into ML foundation model training pipelines.

### 8.2 Vault Engine Export Checkpoint

When an export or transfer (`send_item`) is requested:
1. The vault engine inspects the item's `consent_scope`.
2. If `SCOPE_HUMAN_APPROVAL_ON_EXPORT` is set and the requester is an AI agent, the engine immediately blocks execution and triggers an interactive `ask_human` prompt.
3. If the destination or intended purpose violates the declared scope, the vault engine refuses transmission (`ConsentViolationError`).

---

## 9. Conformance & Test Vectors

A compliant implementation of the S3A Digital Vault specification MUST pass the four normative test vectors specified below. All four vectors are **IMPLEMENTED** and verified in `crates/s3a-spec-vectors` (`vault_transfer_chain`, `vault_delegation_governance`, `vault_key_revocation`, `vault_vc_import`).

### 9.1 Normative Test Vector Suite

| Vector ID | Name | Target Mechanism | Status | Description & Validation Criteria |
|:---|:---|:---|:---:|:---|
| **`VAULT-VEC-01`** | Signed Transfer Chain | Ownership Lineage | **IMPLEMENTED** | Validates a 3-link ownership chain: Genesis Issue by Issuer $\to$ Transfer to Alice $\to$ Transfer to Bob. Verified in `test_vector_vault_transfer_chain`. Pass if Bob resolves as current owner; fail if intermediate link signature or hash is corrupted, or double-send attempted. |
| **`VAULT-VEC-02`** | Delegation Grant + Agent Action | Autonomous Governance | **IMPLEMENTED** | Alice signs a delegation grant to Agent Gamma. Agent Gamma executes `ACTION_WRITE` recording `grant_id`. Agent Gamma then attempts a judgment action (`erasure`). Verified in `test_vector_vault_delegation_governance`. Pass if `ACTION_WRITE` succeeds and judgment action is blocked with `RequiresHumanApproval`. |
| **`VAULT-VEC-03`** | Revoked Key Enforcement | Key Management | **IMPLEMENTED** | Key pair $K_1$ signs a rotation record to $K_2$. An action is subsequently attempted using $K_1$. Verified in `test_vector_vault_key_revocation`. Pass if action after rotation is rejected with `KeyRevokedError` while historical actions signed prior to rotation remain valid. |
| **`VAULT-VEC-04`** | W3C VC / Open Badges 3.0 Import | Interoperability | **IMPLEMENTED** | Imports a standard W3C VC v2.0 Open Badges 3.0 JSON-LD document with `Ed25519Signature2020`. Verified in `test_vector_vault_vc_import`. Pass if issuer signature verifies, raw JSON-LD is stored in Blob tile (68), and query index is correctly created in `VaultItemRecord` (64 / 38). |

---

## 10. Things (Physical & Edge Assets)

S3A Vaults natively manage physical things: embedded sensors, edge gateways, laboratory instruments, diagnostic hardware, and industrial equipment.

### 10.1 Physical Asset Identity & Hardware Anchoring

Every physical thing represented in an S3A Vault possesses an unforgeable cryptographic identity with `owner_kind == VaultOwnerKind::Thing`:

1. **Hardware Root of Trust**:
   - Device keypairs MUST be generated inside and anchored to tamper-resistant hardware:
     - Dedicated Secure Elements (e.g. Microchip ATECC608A/B, NXP SE050, Infineon Optiga Trust M).
     - Microcontroller/SoC embedded Secure Elements (eSE) or Hardware Security Modules (HSM).
     - Firmware TPM 2.0 or PUF (Physically Unclonable Function) keys.
   - Private keys are non-extractable; signing occurs exclusively within the hardware cryptographic engine.
2. **Thing Identity Record (`ThingIdentityRecord`)**:
   - Plain-old-data record (128 bytes, 8-byte aligned) linking hardware telemetry to the vault:

#### Layout (128 Bytes, 8-Byte Aligned, Little-Endian)
| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `thing_id` | Unique 128-bit UUID assigned to the physical device. |
| `16 .. 32` | `[u8; 16]` | `model_id` | Manufacturer hardware model identifier / UUID. |
| `32 .. 48` | `[u8; 16]` | `serial_number` | Hardware serial number / MAC address (UTF-8 or bytes). |
| `48 .. 56` | `u64` | `firmware_version` | Packed semantic version (`major.minor.patch.build`). |
| `56 .. 88` | `[u8; 32]` | `public_key` | 32-byte Ed25519 raw public key anchored to hardware. |
| `88 .. 96` | `u64` | `provisioned_at` | Unix epoch timestamp of vault provisioning. |
| `96 .. 100` | `u32` | `operational_flags` | Bitmask: `0x01 = Calibrated`, `0x02 = Active`, `0x04 = EdgeProfile`. |
| `100 .. 124`| `[u8; 24]` | `_reserved` | Explicit zero padding. |
| `124 .. 128`| `u32` | `record_crc32` | Castagnoli CRC32C over bytes `0..124`. |

### 10.2 Telemetry Binding & Stream Integrity

Telemetry generated by physical things is cryptographically bound to the device identity:
- **Direct Edge Signing**: Each micro-tile or telemetry block written by the thing carries an on-device Ed25519 signature over its payload hash and `prev_seal_hash` chain (per `S3A-EDGE-PROFILE.md`).
- **Tamper Evidence**: When telemetry is ingested into a parent S3A Vault, the vault retains the device's original seal chain. Modifying or splicing sensor samples breaks the mathematical seal chain.
- **Provenance Attestation**: Scientific analyses run against telemetry records can definitively prove the exact instrument serial number, calibration status, and firmware version that produced each data point.

### 10.3 Equipment Maintenance, Calibration & Operational Logs

Equipment lifecycle events are recorded as immutable, append-only records within the vault:
1. **Operating Events**: Device start/stop timestamps, duty cycles, operating temperature envelopes, and runtime counters.
2. **Calibration Certificates**: Signed calibration records containing calibration standard references, drift measurements, and next-calibration due dates.
3. **Fault Codes & Diagnostic Traces**: Hardware exception events, brownout warnings, communication timeouts, and sensor fault telemetry.
4. **Maintenance Attestations**: Service logs signed by certified service technicians (`VaultOwnerKind::Person`) or automated self-test diagnostics (`VaultOwnerKind::Thing`).

### 10.4 Operational Authority & Control Boundaries

To safeguard physical systems against autonomous misbehavior:
1. **Telemetry vs. Control Separation**: The ability of an agent to read telemetry (`ACTION_READ`) never conveys the authority to actuate machinery or alter configuration.
2. **Actuator Command Gating**: Setpoint changes, calibration adjustments, firmware upgrades, and emergency overrides require an explicit Delegation Grant with `ACTION_WRITE` specifically scoping physical control parameters.
3. **Supervisory Review for Physical Impact**: Parameter changes that could cause physical damage, safety hazards, or process invalidation are classified as non-delegable judgment actions and route unconditionally to the **Human Review Checkpoint** (§12.2).

### 10.5 Mutual Attestation & Provisioning with Edge Profile

Physical things executing the lightweight S3A Edge Profile (`S3A-EDGE-PROFILE.md`) interface seamlessly with S3A Vaults:
1. **Provisioning Handshake**:
   - The device presents its `ThingIdentityRecord` and hardware public key.
   - The vault principal signs a `ProvisioningReceiptRecord` binding the device to the vault namespace.
   - The receipt is written to the device's genesis consent/provisioning tile.
2. **Synchronization Protocol**:
   - Devices sync with parent vaults over BLE, USB, Wi-Fi, serial, or LoRa.
   - Inbound `.edge.s3a` micro-tiles are deduplicated by tile hash; duplicate tiles are ignored without error.
   - The vault merges edge micro-tiles into standard 128 KB hyper-tiles while preserving the device's original `prev_seal_hash` chain and device signature intact.

---

## 11. Gate Addresses (Vault Network Addressing & Dialing Protocol)

Every S3A Vault possesses an immutable network address designated as a **Gate Address**. Gates provide a decentralized, transport-agnostic addressing and dialing fabric enabling vaults to discover, authenticate, and exchange sealed records across local radios, store-and-forward relays, and cloud repositories.

### 11.1 Principle: Gates as Network Addresses

In the S3A Vault architecture, a **Gate** is the sovereign network endpoint of a vault instance:
- **Permanent Address**: Every vault has a permanent gate address derived as a cryptographic digest of its master Ed25519 public key.
- **Checksummed & Segmented Representation**: Encoded as a checksummed, human-readable word/segment format (e.g., `s3a-7f9a-c21d-e4b8-93`).
- **Standardized URI Form**: Formatted canonically as:
  ```
  gate://<address>
  ```
  Example: `gate://s3a-7f9a-c21d-e4b8-93`.
- **Mathematical Immutability**: Because the address is bound mathematically to the vault's root public key, it cannot be forged, spoofed, or arbitrarily reassigned.

### 11.2 Gate Names & Signed Directory Records

To support human-friendly discovery without sacrificing permanent cryptographic binding:
1. **Optional Gate Names**:
   - Vaults MAY register human-readable names formatted as:
     ```
     gate://<name>[.<namespace>]
     ```
     Examples: `gate://lab-alpha`, `gate://ashley.research`, `gate://sensor-station-04.biology`.
2. **Signed Directory Records (`GateDirectoryRecord`)**:
   - A gate name is mapped to its underlying gate address and current active endpoints within an append-only, cryptographically signed directory record.
   - **Re-Pointable Names, Permanent Addresses**: Names are re-pointable as an entity changes network hosts, relays, or institutions; addresses remain permanent.
   - **Endpoint Descriptors**: Directs callers to active transports:
     - LRS Server / Cloud REST API (e.g. `https://lrs.institution.edu/s3a`).
     - Store-and-Forward Relay (e.g. `relay://hub.s3a.network:8443`).
     - Local Transports (BLE MAC/UUID, NFC NDEF record, Wi-Fi Direct SSID, USB serial port).

#### GateDirectoryRecord Binary Layout (160 Bytes, 8-Byte Aligned, Little-Endian)
| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `record_id` | Unique 128-bit UUID of this directory publication. |
| `16 .. 48` | `[u8; 32]` | `gate_name` | Null-terminated UTF-8 gate alias (e.g. `lab-alpha.org\0`). |
| `48 .. 80` | `[u8; 32]` | `gate_address` | 32-byte address / raw public key digest. |
| `80 .. 112`| `[u8; 32]` | `endpoint_ref` | SHA-256 digest referencing active transport endpoint list. |
| `112 .. 120`| `u64` | `valid_from` | Unix epoch timestamp of registration validity start. |
| `120 .. 128`| `u64` | `valid_until`| Unix epoch timestamp of registration expiration. |
| `128 .. 132`| `u32` | `directory_seq`| Monotonically increasing sequence number (for re-pointing). |
| `132 .. 156`| `[u8; 24]` | `_reserved` | Explicit zero padding. |
| `156 .. 160`| `u32` | `record_crc32`| Castagnoli CRC32C over bytes `0..156`. |

### 11.3 Dial Flow & Challenge-Response Protocol

When an agent, peer vault, phone reader, or edge instrument initiates communication with a target gate, it executes the normative 5-step **Dial Flow**:

```mermaid
sequenceDiagram
    autonumber
    participant Caller as Caller Vault / Client
    participant Dir as Signed Directory / Relay
    participant Target as Target Vault

    Caller->>Dir: 1. Resolve: Query gate://<name> or lookup gate://<address>
    Dir-->>Caller: Signed GateDirectoryRecord (Address + Endpoints)
    Caller->>Target: 2. Connect: Establish transport session (TLS, BLE, USB)
    Target-->>Caller: Cryptographic Challenge Nonce (32 random bytes)
    Caller->>Target: 3. Challenge-Response: Ed25519 signature proving private key ownership
    Target-->>Caller: Ownership Verified + Target Challenge Nonce (Mutual Auth)
    Caller->>Target: 4. Present Access Code: Submit short-lived signed grant token
    Target->>Target: Validate grant scope, expiry, max use count
    Caller->>Target: 5. Act Within Grant: Synchronize or exchange sealed tiles
```

1. **Resolve**: Caller resolves `gate://<name>` via directory service to obtain the canonical `gate://<address>` and active endpoint connection parameters.
2. **Connect**: Caller establishes physical or logical connection across available network transports (HTTPS, WebSocket, BLE GATT, Wi-Fi Direct, USB serial).
3. **Challenge-Response (Key Ownership Proof)**: Target sends a cryptographic nonce (`[u8; 32]`). The caller returns an Ed25519 signature over the nonce, proving possession of the caller identity private key without disclosing it. Mutual authentication is established symmetrically.
4. **Present Access Code**: Caller presents a valid, short-lived signed Access Code authorizing operations for this session.
5. **Act Within Grant**: Target engine bounds session privileges strictly to the scopes and operations authorized in the Access Code.

### 11.4 Access Codes (Short-Lived Signed Grants)

Access to a vault gate requires a valid, revocable **Access Code**:
- **Definition**: An Access Code is a cryptographically signed capability grant issued by the vault owner to a specific caller identity or session key.
- **Grant Fields**:
  - `target_gate_address`: Destination gate address where code is valid.
  - `allowed_scopes`: Permitted tile types and record masks (e.g. telemetry, learner assertions).
  - `allowed_actions`: Permitted operations (`ACTION_READ`, `ACTION_WRITE`, `ACTION_SEND_ITEM`).
  - `expires_at`: Strict short-lived expiration timestamp (typically 60 seconds to 24 hours).
  - `max_use_count`: Bounded usage counter (single-use or session-capped).
  - `issuer_signature`: Ed25519 signature generated by the granting vault owner.
- **Revocability**: Access codes are revocable on-demand by writing a revocation record to the vault event log.

### 11.5 Offline Relay Delivery & Local Transports

Vault network communication operates robustly across intermittent connectivity and air-gapped environments:
1. **Relay Mailboxes**:
   - When a recipient vault is offline, calling nodes deposit sealed, encrypted `.s3a` tiles into an intermediate store-and-forward relay mailbox.
   - Tiles remain tamper-sealed via their `prev_seal_hash` container chains.
   - When the destination vault comes online, it dials the relay, proves key ownership via challenge-response, downloads queued sealed tiles, and verifies their integrity prior to ingestion.
2. **Local Dialing (Edge Profile Sharing)**:
   - Near-field peer vaults and physical edge instruments dial directly over BLE, NFC, USB, or Wi-Fi Direct.
   - Local transfers execute the standard sharing protocol defined in `S3A-EDGE-PROFILE.md`:
     `Hello` $\to$ `Tile Inventory` $\to$ `Request Missing` $\to$ `Send Sealed Tiles` $\to$ `Verify Hash Chain & Signature` $\to$ `ACK`.

### 11.6 Cryptographic & Privacy Boundaries

The gate network architecture enforces strict non-disclosure invariants:
- **No Private Keys on Infrastructure**: Directory servers, network routers, and store-and-forward relays NEVER hold private keys.
- **Payload Inaccessibility**: Relays and directory operators CANNOT inspect, parse, or read consent-restricted payloads. Payloads remain end-to-end encrypted or protected by subject consent masks.
- **Consent Boundary Enforcement**: Relay mailboxes reject deposits of tiles whose `consent_scope` restricts transit through untrusted third-party relays (`SCOPE_NO_REEXPORT` or `SCOPE_INTERNAL_ENTERPRISE`).

---

## 12. Approval Checkpoints (Supervisory & Operational Governance)

To enforce verifiable, fail-safe governance over autonomous agents and multi-party workflows, S3A Vaults implement formal **Approval Checkpoint Contracts**.

### 12.1 Approval Checkpoint Architecture

Approval checkpoints operate as cryptographic evaluation checkpoints. Every checkpoint invocation, approval, rejection, or escalation is permanently recorded as an immutable Pod struct in the vault event log.

#### ApprovalCheckpointRecord Layout (128 Bytes + 64 Byte Sig = 192 Bytes, 8-Byte Aligned, Little-Endian)
| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `checkpoint_id` | Unique 128-bit UUID of the checkpoint evaluation. |
| `16 .. 18` | `u16` | `checkpoint_type` | Checkpoint category (§12.2). |
| `18 .. 20` | `u16` | `outcome` | `0 = Pending`, `1 = Approved`, `2 = Rejected`, `3 = Escalated`, `4 = TimedOut`. |
| `20 .. 24` | `u32` | `target_action` | Action code requested (e.g. `ACTION_WRITE`, `ACTION_SEND_ITEM`). |
| `24 .. 40` | `[u8; 16]` | `requested_by` | UUID of the requesting agent or actor. |
| `40 .. 56` | `[u8; 16]` | `target_item_or_task` | UUID of the holding, tool call, or task being evaluated. |
| `56 .. 60` | `u32` | `threshold_confidence`| IEEE 754 32-bit float representing required confidence. |
| `60 .. 64` | `u32` | `observed_confidence` | IEEE 754 32-bit float representing model/evaluator score. |
| `64 .. 72` | `u64` | `evaluated_at` | Unix epoch timestamp of evaluation. |
| `72 .. 80` | `u64` | `expires_at` | Unix epoch timestamp of decision expiration. |
| `80 .. 112`| `[u8; 32]` | `approver_pubkey` | 32-byte Ed25519 public key of human reviewer or authority. |
| `112 .. 124`| `[u8; 12]` | `_reserved` | Explicit zero padding. |
| `124 .. 128`| `u32` | `record_crc32` | Castagnoli CRC32C over bytes `0..124`. |
| `128 .. 192`| `[u8; 64]` | `approver_signature` | Ed25519 signature over bytes `0..128` (zeros if pending/timeout). |

### 12.2 The Four Canonical Checkpoint Types

```mermaid
flowchart TD
    Req["Agent Action Request"] --> C1{"Preflight Hallucination Checkpoint"}
    C1 -- Pass --> C2{"Consent & Export Checkpoint"}
    C1 -- Fail / Unverified Ref --> Rej["Rejected / Escalated"]
    C2 -- Pass Scope --> C3{"Tool Execution Checkpoint"}
    C2 -- Scope Block / Training Denied --> Rej
    C3 -- Allowlisted & Valid --> C4{"Judgment & Human Review Checkpoint"}
    C3 -- Sandbox Escape / Disallowed --> Rej
    C4 -- Existence Check Only --> Exec["Execute Action"]
    C4 -- Judgment Call or Low Conf --> Pause["Pause Run -> Ask Human"]
    Pause --> Resume["Interactive Human Sign-Off"]
    Resume --> Exec
```

#### 1. Preflight Hallucination Checkpoint (`CHECKPOINT_PREFLIGHT_HALLUCINATION = 1`)
- **Purpose**: Verifies all citations, claims, external references, and data assertions before an action is committed.
- **Constraints**: AI models are restricted strictly to verifiable existence checks:
  - Assert DOI exists in canonical registries (OpenAlex, Crossref).
  - Verify document metadata matches cited values.
  - Assert retraction status is clear.
  - Verify exact text passage is present in source archive.
- **Checkpoint Failure**: Any hallucinated, unverified, or retracted reference immediately trips the checkpoint, blocking the action and logging an evaluation event.

#### 2. Human Review Checkpoint (`CHECKPOINT_HUMAN_REVIEW = 2`)
- **Purpose**: Supervisory intervention for high-consequence judgment calls and sub-threshold confidence decisions.
- **Trigger Conditions**:
  - Any operation classified under the Non-Delegable Judgment Invariant (§3.2, §12.5).
  - Operational routing decisions where observed confidence falls below the agent's configured threshold (`observed_confidence < threshold_confidence`).
- **Behavior**:
  - The agent loop immediately pauses with status `PausedWaitingHuman`.
  - Execution state is frozen into the vault / sandbox run ledger.
  - The human operator reviews the proposed action and submits a signed decision via `s3a-harness resume`.

#### 3. Consent Checkpoint (`CHECKPOINT_CONSENT = 3`)
- **Purpose**: Enforces subject consent, GDPR, HIPAA, and IRB compliance across transfers, queries, and analytics.
- **Verification Rules**:
  - Inbound and outbound items are checked against their `consent_scope` bitmask.
  - Operations attempting model training without explicit opt-in (`SCOPE_MODEL_TRAINING_FORBIDDEN`) are blocked at the engine layer.
  - If `SCOPE_HUMAN_APPROVAL_ON_EXPORT` is present, export cannot proceed autonomously.

#### 4. Tool Execution Checkpoint (`CHECKPOINT_TOOL_EXECUTION = 4`)
- **Purpose**: Enforces sandbox isolation, tool parameter schemas, and path boundaries before invoking external or local tools.
- **Isolation Rules**:
  - Enforces strict agent tool allowlists configured in `agents/<id>.toml`.
  - Rejects path traversal escapes (all file writes confined to `<root>/exports`).
  - Records cryptographic hashes of tool inputs (`SHA-256(input)`) and tool outputs (`SHA-256(output)`) into the xAPI provenance ledger.

### 12.3 Multi-Signature Quorums

For institutional or irreversible actions, checkpoints MAY require an $M$-of-$N$ multi-signature quorum:
- **Quorum Scenarios**:
  - Irreversible record erasure (`erasure` tile purge).
  - Identity key revocation or master key rotation.
  - Large-scale credential mass-revocation.
  - Multi-institutional dataset exports.
- **Protocol**: A `QuorumCheckpointRecord` collects $M$ independent Ed25519 signatures from authorized organizational keys before transitioning `outcome` from `Pending` to `Approved`.

### 12.4 Timeout & Escalation Policies

1. **Deterministic Expiration (`expires_at`)**: Every pending checkpoint evaluation defines a maximum lifetime.
2. **Fail-Closed Default**: If `expires_at` is reached without required approvals, the outcome transitions to `TimedOut` and the requested action is rejected without execution.
3. **Single-Escalation Rule**: To prevent runaway escalation loops, an agent run is permitted at most **one** automated model escalation (e.g. switching to a stronger model upon becoming stuck). Subsequent failures route directly to the Human Review Checkpoint.

### 12.5 Non-Delegable Judgment Invariant Integration

The core architectural invariant of the S3A Vault guarantees human primacy over subjective scientific, ethical, and legal decisions:

$$\forall a \in \text{Actions}, \; \text{is\_judgment}(a) \implies \text{delegable}(a) = \text{false}$$

- **Non-Delegable Domain Actions**:
  - Scientific claim support assessments (e.g., whether evidence supports an empirical hypothesis).
  - Methodological rigor evaluations.
  - Scientific overstatement and unwarranted extrapolation assessments.
  - Irreversible data purging and subject consent withdrawal.
  - Legal contract / waiver execution.
- **Enforcement Guarantee**: No prompt, configuration file, model parameter, or delegation grant can override this restriction. A vault engine encountering a judgment action MUST route to `ACTION_ASK_HUMAN`.

---

## 13. Tile ID Registry Harmonization & Conflict Analysis

### 13.1 Allocation Band Review

Per `registry.md` and `S3A-TILE-SPEC.md`, tile type IDs are partitioned into strict bands:
- **0 – 32**: Legacy Frozen Wire Assignments.
- **33 – 63**: Reserved for Standard Domain Packs (allocated via spec amendment).
- **64 – 127**: Core Catalog Primitives.

### 13.2 Conflict-Free Mapping Table

The table below demonstrates how the S3A Vault maps to canonical allocations with **zero collisions**:

| Vault Subsystem / Record | Assigned Wire ID | Hex | Owning Band | Registry Status | Conflict Analysis |
|:---|:---:|:---:|:---|:---|:---|
| **Raw Credential JSON-LD** | `68` | `0x0044` | Core Catalog | Active (`blob`) | **Zero Conflict**. Stores uncompressed or LZ4 VC documents cleanly in standard blob chunks. |
| **Holding Metadata & Items**| `64` | `0x0040` | Core Catalog | Active (`record`) | **Zero Conflict**. Tabular Pod item entries stored in core record tiles. |
| **Container & Ledger Seals** | `70` | `0x0046` | Core Catalog | Active (`seal`) | **Zero Conflict**. Tamper-evident hash chaining and OTS anchoring. |
| **Consent & Governance** | `71` | `0x0047` | Core Catalog | Active (`consent`) | **Zero Conflict**. Uses canonical subject consent primitive. |
| **Agent Action Provenance** | `72` | `0x0048` | Core Catalog | Active (`provenance`) | **Zero Conflict**. Multi-agent lineage tracking linking agent tool steps to grants. |
| **Educational Credentials** | `30` | `0x001E` | Core ELR | Active (`elr_credential_assertion`)| **Zero Conflict**. IEEE P2997 / Open Badges index projection. |
| **Dedicated Vault Pack Range**| `36 – 40`| `0x0024..`| Standard Packs | Reserved (Amendable)| **Zero Conflict**. Standard Domain Pack band (33–63) is completely unallocated; reserved for specialized vault types: `36: vault_identity`, `37: vault_delegation`, `38: vault_holding`, `39: vault_transfer`, `40: vault_event_log`, plus approval checkpoint and gate directory records. |

---

## 14. Summary & Architecture Checklist

1. **Native Format**: An S3A Vault is an ordinary `.s3a` archive (`.vault.s3a`); fully compatible with existing compactors, readers, and verification tools.
2. **Key Custody**: Person keys reside in hardware keystores; agent keys reside in the execution harness and are never exposed to language models; organization keys reside in HSMs; thing keys reside in tamper-resistant secure elements.
3. **Bounded Delegation**: Agents execute under signed, bounded delegation grants; judgment actions can never be delegated and route unconditionally to humans.
4. **Credential Standards**: Direct bidirectional translation between W3C Verifiable Credentials / Open Badges 3.0 and S3A storage primitives.
5. **Ownership Provenance**: Cryptographic transfer hash chains verify current item ownership; optional LRS registry checks prevent double-sends.
6. **Things & Physical Assets**: Hardware-anchored identity binding for physical instruments, on-device telemetry signing, equipment lifecycle logs, and seamless mutual attestation with the Edge Profile (`S3A-EDGE-PROFILE.md`).
7. **Gate Addresses & Dialing Protocol**: Every vault possesses an immutable `gate://<address>` network address derived from its public key hash, with re-pointable directory names (`gate://<name>[.<namespace>]`), challenge-response mutual attestation, short-lived access codes, offline relay mailboxes, and edge dialing over BLE/NFC/Wi-Fi per S3A-EDGE-PROFILE.
8. **Approval Checkpoints**: Four canonical checkpoint contracts (Preflight Hallucination, Human Review, Consent, Tool Execution) enforce fail-safe execution, multi-sig quorums, and auditability.
9. **Consent Governance**: Native storage-level consent masks enforce participant privacy, restricting model training and commercial exploitation.
10. **Disaster Recovery**: Guardian device authorization, BIP-39 deterministic recovery phrases, and encrypted snapshot containers with Reed-Solomon parity and OTS anchoring.
11. **Non-Financial Guarantee**: Strictly excludes money, payments, and crypto tokens, focusing exclusively on identity, knowledge, learning credentials, physical assets, supervisory governance, and research data provenance.

---

## 15. Implementation Status & Conformance Audit (Phase 1)

### 15.1 Implementation Component Manifest

Phase 1 (Core Identity, Encrypted Key Storage, Gate Addressing, Bounded Delegation, Transfer Chains, Rotation & Revocation, Backup/Restore, Gate Directory, Harness Integration, and CLI) is fully implemented in Rust:

| Component | Repository Path | Lines of Code | Spec Section | Primary Responsibilities |
|:---|:---|:---:|:---:|:---|
| **Binary Records** | `crates/s3a-vault/src/records.rs` | 560 | §2.1, §3.1, §4.1, §5.1, §6.1, §7.1, §8.1, §10.2, §11.2, §12.2 | Zero-copy Pod records without compiler padding: `VaultIdentityRecord` (96 B), `DelegationGrantRecord` (192 B), `VaultItemRecord` (160 B), `ItemTransferRecord` (240 B), `KeyRotationRecord` (220 B), `RevokedKeyRecord` (64 B), `ThingIdentityRecord` (128 B), `GateDirectoryRecord` (160 B), `ApprovalCheckpointRecord` (192 B), `VaultEventRecord` (80 B). |
| **Key Custody & Storage** | `crates/s3a-vault/src/keystore.rs` | 276 | §2.2, §2.3 | `KeyStore` trait, `MemoryKeyStore`, and `FileKeyStore` encrypted at rest with Argon2id + ChaCha20Poly1305. Private keys are never committed to `.vault.s3a` archives. Zeroize-on-drop for sensitive key material. |
| **Gate Addressing** | `crates/s3a-vault/src/address.rs` | 195 | §11.1 | `GateAddress` derivation (`SHA-256(pubkey)` prefix + CRC32C checksum byte), segmented format `s3a-XXXX-XXXX-XXXX-XXXX-CC`, `gate://` URI parser/validator with single-character typo rejection. |
| **Delegation & Governance** | `crates/s3a-vault/src/delegation.rs` | 241 | §3.2, §12.5 | `create_delegation_grant`, `verify_delegation_grant`, `evaluate_agent_action`. Anti-escalation rules: no self-grant, no widening, limit tracking, expiration. Non-delegable Judgment Invariant: judgment actions route to human approval. |
| **Access Codes** | `crates/s3a-vault/src/access_code.rs` | 236 | §11.3 | Short-lived signed authorization tokens (`s3a-ac.<hex>`), Ed25519 signatures, `AccessCodeSessionManager` (expiry, max uses, revocation). |
| **Transfers & Ownership** | `crates/s3a-vault/src/transfer.rs` | 233 | §5.1–§5.4 | `compute_transfer_hash`, `create_transfer_record`, `verify_transfer_record`, `resolve_chain_ownership`, double-send detection via lineage verification. |
| **Key Rotation & Revocation** | `crates/s3a-vault/src/rotation.rs` | 149 | §6.1, §6.2 | `create_key_rotation_record`, `verify_key_rotation_record`, `RevocationRegistry`. Enforces invariant: actions at/after `revoked_at` fail (`KeyRevokedError`), prior actions remain valid. |
| **Encrypted Backup & Restore** | `crates/s3a-vault/src/backup.rs` | 137 | §8.2 | `create_backup` and `restore_backup` using Argon2id + ChaCha20Poly1305 authenticated envelope encryption. |
| **Vault Storage Engine** | `crates/s3a-vault/src/vault.rs` | 395 | §1.2, §4, §5, §7 | `Vault::create`, `Vault::open`, `add_item`, `add_grant`, `add_transfer`, `add_key_rotation`, `log_event`, `gate_address`, `gate_uri`. Reads/writes standard S3A archive tiles losslessly. |
| **Gate Directory & Relay Service** | `crates/s3a-lrs-server/src/gate_directory.rs` | 434 | §11.2, §11.4 | `GateDirectoryService` behind `--gate-directory`: signed name-to-address registration, challenge-response nonce verification, store-and-forward offline relay mailbox. |
| **Harness Delegation Enforcement** | `crates/s3a-harness/src/tools.rs` | L46–L105 | §3.2, §12.5 | Wire `require_grant: bool` and `grant_record: Option<DelegationGrantRecord>` into tool execution. Automatic blockage of judgment actions and verification of active grants. |
| **CLI Binary** | `crates/s3a-vault/src/main.rs` | 370 | §1.2, §11 | CLI commands: `new`, `show`, `address`, `grant`, `revoke`, `code`, `send`, `receive`, `rotate`, `backup`, `restore`, `dial`. |
| **Conformance Vectors** | `crates/s3a-spec-vectors/src/lib.rs` | L1587–1870, L2100–2135 | §9.1 | Conformance test vectors `VAULT-VEC-01` through `VAULT-VEC-04` with companion expected JSON schemas and automated verification. |

### 15.2 Test Matrix & Verification Results

All tests across the entire repository workspace pass with zero failures:

| Test Suite | Test Location | Test Count | Result | Scope / Invariants Tested |
|:---|:---|:---:|:---:|:---|
| `s3a-vault` Core Suite | `crates/s3a-vault/src/` | 9 | **PASS** | Pod record alignment/sizes; GateAddress round-trip & typo rejection; delegation grant validation, self-grant rejection, non-widening, judgment blocking; Vault archive create/open; key rotation & revocation enforcement; access code session issuance & consumption; transfer chain ownership & double-send detection; encrypted backup/restore; FileKeyStore passphrase encryption. |
| `s3a-lrs-server` Gate Directory | `crates/s3a-lrs-server/src/gate_directory.rs` | 17 | **PASS** | Challenge-response authentication, replay attack rejection, duplicate registration conflict handling, endpoint resolution, store-and-forward relay delivery to offline vaults. |
| `s3a-harness` Delegation Suite | `crates/s3a-harness/src/` | 54 | **PASS** | Tool execution denied when grant required but missing; active grant verified; judgment action blocked with `RequiresHumanApproval`; agent runtime tool provenance logging. |
| `s3a-spec-vectors` Vault Suite | `crates/s3a-spec-vectors/src/lib.rs` | 4 | **PASS** | `VAULT-VEC-01` (3-link transfer lineage), `VAULT-VEC-02` (delegation grant + agent action + judgment block), `VAULT-VEC-03` (key rotation & revocation enforcement), `VAULT-VEC-04` (W3C VC / Open Badges 3.0 import in Blob tile 68). |
| Full Workspace Suite | Repository workspace | 219 | **PASS** | Core engine (100), scholar pack (41), edge profile (15), SIMD (13), WASM snapshot (5), wearable (3), CLI, CABI, and integration suites. |
| **Workspace Total** | `cargo test --workspace` | **303** | **PASS** | **100% green across all crates (0 failures, 0 errors).** |

### 15.3 Specification Clarifications & Wire Harmonization Notes

1. **Zero-Padding Alignment under `bytemuck::Pod`**:
   To prevent undefined layout behavior across architectures, Pod structs must not have compiler-inserted padding. Fields whose offsets are not naturally 8-byte aligned (e.g. `created_at: [u8; 8]` at offset 84 in `VaultIdentityRecord`, or `acquired_at: [u8; 8]` at offset 116 in `VaultItemRecord`) are represented as fixed byte arrays with dedicated `get_*() -> u64` and `set_*(u64)` accessors.
2. **1-Based vs. 0-Based Tile Indexing**:
   The S3A storage engine assigns 1-based tile IDs on commit (`committed_tile_id`), whereas random-access readers index tiles 0-based. In `VaultItemRecord.payload_ref`, `payload_tile_index()` records the 0-based tile index matching `MmapReader::get_tile()`.
3. **Blob Tile Allocation (Tile 68)**:
   Raw JSON-LD documents for W3C Verifiable Credentials and Open Badges 3.0 are stored in `TileType::BLOB` (Wire ID `68` / `0x0044`), with index records stored in `TileType::RECORD` (Wire ID `64`) and/or `TileType::ELR_CREDENTIAL_ASSERTION` (Wire ID `30`).

---

## 16. Code Walkthrough & Usage Guide

### 16.1 Creating a Vault & Primary Identity

```rust
use s3a_vault::{Vault, VaultOwnerKind, FileKeyStore};

// 1. Initialize encrypted keystore (Argon2id + ChaCha20Poly1305)
let mut keystore = FileKeyStore::open_or_create("keys.enc", "passphrase123")?;

// 2. Create a new sovereign vault archive
let vault = Vault::create("alice.vault.s3a", "Alice's Vault", VaultOwnerKind::Person, &mut keystore)?;

// 3. Inspect identity and Gate network address
let primary = vault.primary_identity().unwrap();
let gate_addr = vault.gate_address()?;
println!("Vault initialized. Gate Address: {}", gate_addr.to_uri());
// Output: gate://s3a-02a8-f9b1-7c4d-0e31-89
```

### 16.2 Bounded Delegation Grants & Non-Delegable Judgment Invariant

```rust
use s3a_vault::{create_delegation_grant, evaluate_agent_action, is_judgment_action, ACTION_READ, ACTION_WRITE};

// Granter creates bounded grant for Agent Gamma
let grant = create_delegation_grant(
    grant_id,
    alice_id,
    VaultOwnerKind::Person,
    agent_gamma_id,
    agent_gamma_pubkey,
    ACTION_READ | ACTION_WRITE,
    0x0001, // Scopes
    100,    // Max 100 tool invocations
    now,
    now + 86400, // Valid 24 hours
    None,
    &keystore,
)?;

// Agent executes allowable action
assert!(evaluate_agent_action(&grant, ACTION_WRITE, "record_write", now + 10, 1).is_ok());

// Agent attempts judgment action (e.g. data erasure) -> BLOCKED
assert!(is_judgment_action("erasure"));
let res = evaluate_agent_action(&grant, ACTION_WRITE, "erasure", now + 10, 1);
assert!(matches!(res, Err(s3a_vault::VaultError::RequiresHumanApproval(_))));
```

### 16.3 Ownership Lineage & Double-Send Prevention

```rust
use s3a_vault::{create_transfer_record, resolve_chain_ownership};

// Transfer Item from Issuer -> Alice -> Bob
let transfer_1 = create_transfer_record(t1_id, item_hash, issuer_id, alice_pk, t1_time, [0u8; 32], 0, &keystore)?;
let link1_hash = transfer_1.compute_transfer_hash();

let transfer_2 = create_transfer_record(t2_id, item_hash, alice_id, bob_pk, t2_time, link1_hash, 0, &keystore)?;

// Resolve current owner public key
let chain = vec![transfer_1, transfer_2];
let current_owner = resolve_chain_ownership(&issuer_pk, &chain)?;
assert_eq!(current_owner, bob_pk);
```

### 16.4 CLI Usage Guide

The `s3a-vault` CLI provides convenient access to all vault features:

```bash
# Create a new vault
s3a-vault new alice.vault.s3a --name "Alice" --kind person --keys keys.enc --passphrase "secret"

# Display vault summary, primary identity, and gate address
s3a-vault show alice.vault.s3a

# Issue a delegation grant to an agent
s3a-vault grant alice.vault.s3a --agent-key <HEX> --actions "read,write" --expires-in 3600 --keys keys.enc --passphrase "secret"

# Generate a short-lived access code token
s3a-vault code alice.vault.s3a --scope "credential_verify" --actions "read" --uses 1 --ttl 300 --keys keys.enc --passphrase "secret"

# Dial a remote vault through the Gate Directory
s3a-vault dial gate://lab.mit.edu --directory http://127.0.0.1:8080 --keys keys.enc --passphrase "secret"

# Create encrypted disaster-recovery backup
s3a-vault backup alice.vault.s3a alice.backup.enc --passphrase "recovery-passphrase"

# Restore from backup
s3a-vault restore alice.backup.enc restored.vault.s3a --passphrase "recovery-passphrase"
```


---

## 17. Signed Share Links & QR Codes (`vault share` / `vault open`)

Additive and opt-in. Share tokens live outside archives; no S3A on-disk format, tile, or seal-hash rule changes.

### 17.1 Token

The link text and the QR payload are byte-identical:

```
gate://<issuer address>/share/<base64url(canonical || signature)>
```

base64url is RFC 4648 §5 without padding; decoders MUST reject non-alphabet characters and non-zero trailing bits (RFC 4648 §3.3, §3.5). Canonical bytes, fixed order, integers big-endian, strings `u16 length || UTF-8`:

| Field | Size | Notes |
|---|---|---|
| domain | 16 | `"S3A-VAULT-SHARE\0"` — separates these signatures from seals, grants and access codes |
| version | 1 | `1` |
| sig_alg | str | ID from the algorithm registry (registry.md §6); only `ed25519` is accepted for share tokens |
| issuer_pubkey | 32 | Ed25519 public key |
| address | 9 | `SHA-256(pubkey)[0..8] || checksum` (§11); MUST equal the value derived from `issuer_pubkey` |
| item_hash | 32 | SHA-256 of the shared item content |
| issued_at, expires_at | 8 + 8 | unix seconds; lifetime ≤ 30 days |
| nonce | 16 | random |
| audience | 1 (+str) | `0` none, or `1` + recipient gate address |

The signature is Ed25519 (RFC 8032 §5.1.6) over the canonical bytes, verified with `verify_strict` (RFC 8032 §5.1.7 checks). The QR symbol is ISO/IEC 18004 byte mode, error-correction level M.

### 17.2 `vault open` order of checks

Every failure prints `REJECTED: <reason>` and exits 3; nothing is acted on.

1. Parse: link host, embedded address and `issuer_pubkey` must all agree (self-certifying address).
2. `sig_alg` passes `check_readable` and is `ed25519` (`none`, reserved and unknown IDs reject).
3. Ed25519 signature over the canonical bytes.
4. Lifetime ≤ 30 days; `issued_at ≤ now + 300 s`; `now < expires_at`.
5. Audience, if present, equals the opener's own gate address (`--as <vault>`); otherwise reject.
6. Item hash equals the supplied item (`--item`/`--item-hash`), when supplied.
7. Replay: `(issuer_pubkey, nonce)` must not be in the local seen-store (`~/.s3a/vault/share-seen.json`); it is recorded only after steps 1–6 pass, and pruned after expiry.

On success it prints `VERIFIED`, the issuer petname **only if the full 32-byte key is pinned** in `contacts.json` (`vault contact add`), otherwise `UNKNOWN ISSUER`, and the gate address and SHA-256 key fingerprint.

### 17.3 Threat model

| Threat | Result |
|---|---|
| Forged code (attacker signs with their own key but claims the victim's key/address) | Signature check fails → REJECTED |
| Swapped sticker (attacker's own valid code pasted over the real one) | Verifies, but shows the attacker's address and `UNKNOWN ISSUER`, never the victim's petname |
| Link host edited to the victim's address | Host ≠ derived address → REJECTED |
| Tampered payload (item, expiry, audience, alg, nonce) | Signature fails → REJECTED |
| Replay of a used code | Seen-store → REJECTED |
| Expired / future-dated code | REJECTED |
| Code for another recipient | Audience mismatch → REJECTED |
| Image with several QR symbols (partial overlay) | REJECTED; the decoder will not guess |

### 17.4 Honest limits

- A swapped sticker cannot be detected cryptographically: it is a valid code from someone else. Protection depends on the user reading the issuer line; unpinned issuers are shown as `UNKNOWN ISSUER`.
- The gate address is a 64-bit hash. A targeted second-preimage search (~2^64 work) could produce a key with the same address. Petnames are therefore bound to the full key, not the address.
- The replay store is local to one device. A code can be opened once on each device unless `audience` is set and the item is single-use at a higher layer.
- Time checks trust the local clock.
- A token proves who signed it and what item hash it names. It does not prove the issuer owns or still holds the item, and it is not a revocation check.
- Tokens are not encrypted; anyone who sees the code learns the issuer key, item hash and audience.
- Item hash is only compared when the item is supplied to `vault open`.
- The tests cover synthetic phone-screenshot images (scaled, blurred, low contrast). They do not cover real camera photos with perspective distortion.

### 17.5 References

- RFC 8032, Edwards-Curve Digital Signature Algorithm (EdDSA): https://www.rfc-editor.org/rfc/rfc8032
- RFC 4648, The Base16, Base32, and Base64 Data Encodings (§5 base64url): https://www.rfc-editor.org/rfc/rfc4648
- ISO/IEC 18004:2015 (since withdrawn and revised by ISO/IEC 18004:2024), QR Code bar code symbology specification: https://www.iso.org/standard/62021.html
