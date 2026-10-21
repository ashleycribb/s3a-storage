//! Simulated agent identity, key generation, and commit authoring.

use ed25519_dalek::{Signer, SigningKey};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use s3a_core::{TelemetryRecord, TileType};
use s3a_pack_scholar::records::{ScholarNoteClaimRecord, CLAIM_KIND_EMPIRICAL};
use s3a_pack_scholar::ScholarTileType;
use s3a_vault::address::GateAddress;
use s3a_vault::records::DelegationGrantRecord;
use s3a_vcs::records::{VcsCommitRecord, VCS_COMMIT_MESSAGE_MAX};

use crate::grant::issue_agent_grant;
use crate::merge::RepoRecordEntry;

/// Prepared commit ready for push submission by an agent.
#[derive(Debug, Clone)]
pub struct AgentCommitBundle {
    pub commit: VcsCommitRecord,
    pub records: Vec<RepoRecordEntry>,
    pub raw_tile_type: u32,
    pub raw_tile_payload: Vec<u8>,
    pub grant: DelegationGrantRecord,
    pub paths: Vec<String>,
    pub is_claim_conflict: bool,
    pub is_grant_violation: bool,
    pub is_overlapping_edit: bool,
    pub is_intentional_bug: bool,
    pub is_repair_fix: bool,
}

/// Simulated concurrent agent with Ed25519 vault identity and scoped delegation grant.
pub struct SimulatedAgent {
    pub index: usize,
    pub id: [u8; 16],
    pub signing_key: SigningKey,
    pub pubkey: [u8; 32],
    pub gate_address: GateAddress,
    pub grant: DelegationGrantRecord,
    pub allowed_prefix: String,
}

impl SimulatedAgent {
    /// Creates a new simulated agent with independent keypair, gate:// address, and signed grant.
    pub fn new(index: usize, root_key: &SigningKey) -> Self {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        // Mix index for deterministic uniqueness per seed
        seed[0..8].copy_from_slice(&(index as u64).to_le_bytes());
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey = signing_key.verifying_key().to_bytes();
        let gate_address = GateAddress::from_public_key(&pubkey);

        let mut id = [0u8; 16];
        id[0..8].copy_from_slice(&(index as u64).to_le_bytes());
        id[8..16].copy_from_slice(&pubkey[0..8]);

        let allowed_prefix = format!("records/agent_{}/", index);
        let grant = issue_agent_grant(
            root_key, id, pubkey, 50, // max ops
            0xFFFF, 86400, // 24h
        );

        Self {
            index,
            id,
            signing_key,
            pubkey,
            gate_address,
            grant,
            allowed_prefix,
        }
    }

    /// Prepares a small signed commit according to distribution parameters:
    /// - ~92% disjoint records (partitioned by agent)
    /// - ~5% deliberately overlapping same-record edits
    /// - ~1% claim/evidence disagreements
    /// - ~2% unauthorized grant attempts
    pub fn prepare_commit(&self, seq: usize, parent_seal: [u8; 32]) -> AgentCommitBundle {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Determine commit behavior based on agent index and sequence
        // Distinct distribution modulo 100
        let slot = (self.index * 17 + seq * 23) % 100;

        let is_intentional_bug = self.index == 7 && seq == 0;
        let is_repair_fix = self.index == 7 && seq == 1;

        let is_claim_conflict = !is_intentional_bug && !is_repair_fix && slot == 42; // ~1%
        let is_grant_violation =
            !is_intentional_bug && !is_repair_fix && (slot == 13 || slot == 88); // ~2%
        let is_overlapping_edit = !is_intentional_bug
            && !is_repair_fix
            && !is_claim_conflict
            && !is_grant_violation
            && (slot >= 50 && slot < 55); // ~5%

        let (tile_type, raw_payload, records, paths, msg) = if is_intentional_bug {
            // Intentional bug: Agent 7 injects a TypeScript error into components/MatchCard.tsx
            let rec_id = "match_card_score_type_bug";
            let rec = TelemetryRecord::new(now, (self.index as u32) + 1, 700, 42.0);
            let payload = bytemuck::bytes_of(&rec).to_vec();
            let mut h = Sha256::default();
            h.update(&payload);
            let payload_hash: [u8; 32] = h.finalize().into();

            let rec_entry = RepoRecordEntry {
                record_id: rec_id.to_string(),
                tile_type: TileType::TELEMETRY,
                type_name: "TELEMETRY".to_string(),
                payload_hash,
                summary: "Agent 7 modifies MatchCard with intentional type error: score property string instead of number (repair demo)".to_string(),
                committed_timestamp: now,
                commit_signature: [0u8; 64],
                author_gate_address: self.gate_address.to_uri(),
            };

            (
                TileType::TELEMETRY,
                payload,
                vec![rec_entry],
                vec![format!("{}components/MatchCard.tsx", self.allowed_prefix)],
                "Agent 7 modifies MatchCard with intentional type error: score property string instead of number (repair demo)".to_string(),
            )
        } else if is_repair_fix {
            // Repair fix: Agent 7 resolves the type error on its shadow branch
            let rec_id = "match_card_score_type_fix";
            let rec = TelemetryRecord::new(now, (self.index as u32) + 1, 701, 42.0);
            let payload = bytemuck::bytes_of(&rec).to_vec();
            let mut h = Sha256::default();
            h.update(&payload);
            let payload_hash: [u8; 32] = h.finalize().into();

            let rec_entry = RepoRecordEntry {
                record_id: rec_id.to_string(),
                tile_type: TileType::TELEMETRY,
                type_name: "TELEMETRY".to_string(),
                payload_hash,
                summary: "fix(MatchCard): Agent 7 resolves type error in score property (repairs shadow branch)".to_string(),
                committed_timestamp: now,
                commit_signature: [0u8; 64],
                author_gate_address: self.gate_address.to_uri(),
            };

            (
                TileType::TELEMETRY,
                payload,
                vec![rec_entry],
                vec![format!("{}components/MatchCard.tsx", self.allowed_prefix)],
                "fix(MatchCard): Agent 7 resolves type error in score property (repairs shadow branch)".to_string(),
            )
        } else if is_claim_conflict {
            // Claim / Evidence Conflict on Brakets match result in fixtures
            let claim_id = "fixture_match_result_claim_open_a";
            let (winner_team, confidence) = if self.index % 2 == 0 {
                ("Solar Flare", 0.98)
            } else {
                ("Vortex Gaming", 0.95)
            };
            let finding = format!(
                "Agent {} empirical report: {} wins Opening Cashout A (confidence {:.2})",
                self.index, winner_team, confidence
            );
            let claim = ScholarNoteClaimRecord::new(
                1,
                (self.index as u64) + 1,
                [0, 100],
                1,
                confidence,
                CLAIM_KIND_EMPIRICAL,
                &finding,
            );
            let payload = bytemuck::bytes_of(&claim).to_vec();
            let mut h = Sha256::default();
            h.update(&payload);
            let payload_hash: [u8; 32] = h.finalize().into();

            let rec_entry = RepoRecordEntry {
                record_id: claim_id.to_string(),
                tile_type: ScholarTileType::NOTE_CLAIMS,
                type_name: "SCHOLAR_NOTE_CLAIMS".to_string(),
                payload_hash,
                summary: finding.clone(),
                committed_timestamp: now,
                commit_signature: [0u8; 64], // will be filled below
                author_gate_address: self.gate_address.to_uri(),
            };

            let path = format!("{}match_result_claim", self.allowed_prefix);
            (
                ScholarTileType::NOTE_CLAIMS,
                payload,
                vec![rec_entry],
                vec![path],
                format!(
                    "Agent {} asserts match result in fixtures: {}",
                    self.index, finding
                ),
            )
        } else if is_grant_violation {
            // Commit touching forbidden path outside grant
            let forbidden_path = "system/unauthorized/global_security_override".to_string();
            let rec = TelemetryRecord::new(now, (self.index as u32) + 1, 999, 1337.0);
            let payload = bytemuck::bytes_of(&rec).to_vec();
            let mut h = Sha256::default();
            h.update(&payload);
            let payload_hash: [u8; 32] = h.finalize().into();

            let rec_entry = RepoRecordEntry {
                record_id: "sec_override_999".to_string(),
                tile_type: TileType::TELEMETRY,
                type_name: "TELEMETRY".to_string(),
                payload_hash,
                summary: format!("Tamper attempt by agent {}", self.index),
                committed_timestamp: now,
                commit_signature: [0u8; 64],
                author_gate_address: self.gate_address.to_uri(),
            };

            (
                TileType::TELEMETRY,
                payload,
                vec![rec_entry],
                vec![forbidden_path],
                format!("Agent {} illegal commit outside grant", self.index),
            )
        } else if is_overlapping_edit {
            // Deliberately colliding edits on Brakets bracket colors or seeding fixtures
            let (shared_key, summary_text, msg_text) = if (self.index % 2) == 0 {
                (
                    "brakets_design_tokens_color_teal".to_string(),
                    format!(
                        "Agent {} adjusts bracket primary teal in design-tokens.json (seq {})",
                        self.index, seq
                    ),
                    format!(
                        "Agent {} adjusts bracket colors in design tokens",
                        self.index
                    ),
                )
            } else {
                (
                    "brakets_fixtures_seeding_order".to_string(),
                    format!(
                        "Agent {} adjusts seeding fixtures in bracket-fixtures.ts (seq {})",
                        self.index, seq
                    ),
                    format!("Agent {} adjusts seeding fixtures", self.index),
                )
            };
            let val = (self.index * 1000 + seq) as f64;
            let rec = TelemetryRecord::new(now, (self.index as u32) + 1, 42, val);
            let payload = bytemuck::bytes_of(&rec).to_vec();
            let mut h = Sha256::default();
            h.update(&payload);
            let payload_hash: [u8; 32] = h.finalize().into();

            let rec_entry = RepoRecordEntry {
                record_id: shared_key.clone(),
                tile_type: TileType::TELEMETRY,
                type_name: "TELEMETRY".to_string(),
                payload_hash,
                summary: summary_text,
                committed_timestamp: now,
                commit_signature: [0u8; 64],
                author_gate_address: self.gate_address.to_uri(),
            };

            let path = format!("{}bracket_edit_{}", self.allowed_prefix, seq);
            (
                TileType::TELEMETRY,
                payload,
                vec![rec_entry],
                vec![path],
                msg_text,
            )
        } else {
            // Disjoint edit unique to this agent: match-card copy, local team notes, styling
            let rec_id = format!("rec_match_card_{}_{}", self.index, seq);
            let rec =
                TelemetryRecord::new(now, (self.index as u32) + 1, seq as u32, (seq * 10) as f64);
            let payload = bytemuck::bytes_of(&rec).to_vec();
            let mut h = Sha256::default();
            h.update(&payload);
            let payload_hash: [u8; 32] = h.finalize().into();

            let rec_entry = RepoRecordEntry {
                record_id: rec_id.clone(),
                tile_type: TileType::TELEMETRY,
                type_name: "TELEMETRY".to_string(),
                payload_hash,
                summary: format!("Agent {} adjusts match-card copy seq {}", self.index, seq),
                committed_timestamp: now,
                commit_signature: [0u8; 64],
                author_gate_address: self.gate_address.to_uri(),
            };

            let path = format!("{}{}", self.allowed_prefix, rec_id);
            (
                TileType::TELEMETRY,
                payload,
                vec![rec_entry],
                vec![path],
                format!("Agent {} adjusts match-card copy (seq {})", self.index, seq),
            )
        };

        // Compute prospective seal hash
        let mut seal_hasher = Sha256::default();
        seal_hasher.update(b"S3A_PROSPECTIVE_SEAL");
        seal_hasher.update(&parent_seal);
        seal_hasher.update(&(self.index as u64).to_le_bytes());
        seal_hasher.update(&(seq as u64).to_le_bytes());
        seal_hasher.update(&raw_payload);
        let prospective_seal: [u8; 32] = seal_hasher.finalize().into();

        // Sign commit
        let env = VcsCommitRecord::compute_envelope_bytes(
            &prospective_seal,
            1,
            &[parent_seal, [0u8; 32]],
            now,
            &msg,
        );
        let commit_id = VcsCommitRecord::compute_commit_id(&env);
        let sig: ed25519_dalek::Signature = self.signing_key.sign(&env);

        let mut msg_bytes = [0u8; VCS_COMMIT_MESSAGE_MAX];
        let mb = msg.as_bytes();
        let m_len = mb.len().min(VCS_COMMIT_MESSAGE_MAX);
        msg_bytes[..m_len].copy_from_slice(&mb[..m_len]);

        let commit = VcsCommitRecord {
            commit_id,
            seal_hash: prospective_seal,
            author_pubkey: self.pubkey,
            author_address_prefix: self.gate_address.prefix,
            author_checksum: self.gate_address.checksum,
            parent_count: 1,
            _pad: [0u8; 6],
            parent_seal_hashes: [parent_seal, [0u8; 32]],
            committed_timestamp: now,
            signature: sig.to_bytes(),
            message_bytes: msg_bytes,
        };

        // Inject the author's signature into the record entries for tie-breaking
        let mut final_records = records;
        for r in &mut final_records {
            r.commit_signature = sig.to_bytes();
        }

        AgentCommitBundle {
            commit,
            records: final_records,
            raw_tile_type: tile_type,
            raw_tile_payload: raw_payload,
            grant: self.grant,
            paths,
            is_claim_conflict,
            is_grant_violation,
            is_overlapping_edit,
            is_intentional_bug,
            is_repair_fix,
        }
    }
}
