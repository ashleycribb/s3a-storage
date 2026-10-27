//! Signed committee decisions (Scholar pack Tile 96, `CommitteeDecisionRecord`, 160 bytes).
//!
//! A committee decision is a *human judgment on submitted work*, not student activity.
//! It therefore lives in its own pack tile, separate from the Tile 10/11 trace tiles,
//! and is **not** gated by the student's tracing consent (Tile 71). Withdrawing consent
//! never marks committee decisions as withdrawn.
//!
//! Integrity:
//! * every record carries `prev_hash` = SHA-256 of the previous record's 160 bytes
//!   (all-zero for the first record), forming an append-only hash chain;
//! * every record carries an Ed25519 signature by the reviewer's key over
//!   [`CommitteeDecisionRecord::signing_message`] (domain tag + first 96 bytes).
//!
//! Key handling: this library never generates, stores, or persists private keys. The
//! calling application (e.g. the Scholar Explorer web app, backed by the reviewer's vault)
//! holds the reviewer key, signs the message, and passes only the public key and
//! signature in. The library only verifies.
//!
//! Comments are not stored in plain text: only a domain-separated SHA-256 commitment
//! (first 16 bytes) is kept, so the app can prove a displayed comment is the one signed.
//! Verdicts are never suggested or pre-filled by this module.

use bytemuck::{bytes_of, Pod, Zeroable};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Maximum comment length accepted (bytes of UTF-8).
pub const COMMITTEE_COMMENT_MAX_BYTES: usize = 4096;
/// Current record layout version.
pub const COMMITTEE_DECISION_LAYOUT_V1: u32 = 1;
/// Domain-separation tag prepended to the signed bytes.
pub const COMMITTEE_SIGNING_DOMAIN: &[u8] = b"S3A-SCHOLAR-COMMITTEE-DECISION-v1\0";

/// Committee verdict. `0` / unknown values are rejected.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitteeVerdict {
    Approve = 1,
    Revise = 2,
    Reject = 3,
}

impl CommitteeVerdict {
    pub fn from_u32(v: u32) -> Result<Self, CommitteeError> {
        match v {
            1 => Ok(Self::Approve),
            2 => Ok(Self::Revise),
            3 => Ok(Self::Reject),
            other => Err(CommitteeError::InvalidVerdict(other)),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Revise => "revise",
            Self::Reject => "reject",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitteeError {
    InvalidVerdict(u32),
    EmptyReviewerId,
    EmptySubmissionId,
    CommentTooLong(usize),
    InvalidPublicKey,
    InvalidSignature,
    ChainBroken(usize),
    LayoutVersion(u32),
}

impl core::fmt::Display for CommitteeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidVerdict(v) => {
                write!(
                    f,
                    "Invalid verdict {v}: expected 1=Approve, 2=Revise, 3=Reject"
                )
            }
            Self::EmptyReviewerId => write!(f, "Reviewer id must not be empty"),
            Self::EmptySubmissionId => write!(f, "Submission id must not be empty"),
            Self::CommentTooLong(n) => write!(
                f,
                "Comment is {n} bytes; maximum is {COMMITTEE_COMMENT_MAX_BYTES}"
            ),
            Self::InvalidPublicKey => write!(f, "Reviewer public key is not a valid Ed25519 key"),
            Self::InvalidSignature => write!(f, "Committee decision signature does not verify"),
            Self::ChainBroken(i) => write!(f, "Committee decision hash chain broken at index {i}"),
            Self::LayoutVersion(v) => write!(f, "Unsupported committee decision layout {v}"),
        }
    }
}

impl std::error::Error for CommitteeError {}

fn tagged_hash16(tag: &[u8], data: &[u8]) -> [u8; 16] {
    let mut h = Sha256::new();
    h.update(tag);
    h.update((data.len() as u64).to_le_bytes());
    h.update(data);
    let d = h.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&d[..16]);
    out
}

/// Commitment for a submission / object identifier.
pub fn committee_submission_hash(submission_id: &str) -> [u8; 16] {
    tagged_hash16(b"s3a.committee.submission.v1", submission_id.as_bytes())
}
/// Commitment for a reviewer identifier.
pub fn committee_reviewer_hash(reviewer_id: &str) -> [u8; 16] {
    tagged_hash16(b"s3a.committee.reviewer.v1", reviewer_id.as_bytes())
}
/// Commitment for a free-text comment (empty comment allowed).
pub fn committee_comment_hash(comment: &str) -> [u8; 16] {
    tagged_hash16(b"s3a.committee.comment.v1", comment.as_bytes())
}

/// SIGNED COMMITTEE DECISION RECORD (160 bytes, 8-byte aligned, Pod).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct CommitteeDecisionRecord {
    pub submission_hash: [u8; 16], // 0..16
    pub reviewer_hash: [u8; 16],   // 16..32
    pub comment_hash: [u8; 16],    // 32..48
    pub prev_hash: [u8; 32],       // 48..80 SHA-256 of previous record (zeros for first)
    pub timestamp_sec: u64,        // 80..88
    pub verdict: u32,              // 88..92
    pub layout_version: u32,       // 92..96
    pub signature: [u8; 64],       // 96..160 Ed25519 over signing_message()
}

impl Default for CommitteeDecisionRecord {
    fn default() -> Self {
        Self::zeroed()
    }
}

const SIGNED_LEN: usize = 96;

impl CommitteeDecisionRecord {
    pub const SIZE: usize = 160;

    /// Builds an *unsigned* record after validating inputs.
    pub fn new_unsigned(
        verdict: u32,
        reviewer_id: &str,
        submission_id: &str,
        comment: &str,
        timestamp_sec: u64,
        prev_hash: [u8; 32],
    ) -> Result<Self, CommitteeError> {
        CommitteeVerdict::from_u32(verdict)?;
        if reviewer_id.trim().is_empty() {
            return Err(CommitteeError::EmptyReviewerId);
        }
        if submission_id.trim().is_empty() {
            return Err(CommitteeError::EmptySubmissionId);
        }
        if comment.len() > COMMITTEE_COMMENT_MAX_BYTES {
            return Err(CommitteeError::CommentTooLong(comment.len()));
        }
        Ok(Self {
            submission_hash: committee_submission_hash(submission_id),
            reviewer_hash: committee_reviewer_hash(reviewer_id),
            comment_hash: committee_comment_hash(comment),
            prev_hash,
            timestamp_sec,
            verdict,
            layout_version: COMMITTEE_DECISION_LAYOUT_V1,
            signature: [0u8; 64],
        })
    }

    /// Exact bytes the reviewer must sign (domain tag || record bytes 0..96).
    pub fn signing_message(&self) -> Vec<u8> {
        let mut m = Vec::with_capacity(COMMITTEE_SIGNING_DOMAIN.len() + SIGNED_LEN);
        m.extend_from_slice(COMMITTEE_SIGNING_DOMAIN);
        m.extend_from_slice(&bytes_of(self)[..SIGNED_LEN]);
        m
    }

    /// SHA-256 of all 160 bytes; the next record's `prev_hash`.
    pub fn record_hash(&self) -> [u8; 32] {
        Sha256::digest(bytes_of(self)).into()
    }

    pub fn verdict_enum(&self) -> Result<CommitteeVerdict, CommitteeError> {
        CommitteeVerdict::from_u32(self.verdict)
    }

    /// Verifies verdict, layout and the Ed25519 signature against `reviewer_pubkey`.
    pub fn verify_signature(&self, reviewer_pubkey: &[u8; 32]) -> Result<(), CommitteeError> {
        self.verdict_enum()?;
        if self.layout_version != COMMITTEE_DECISION_LAYOUT_V1 {
            return Err(CommitteeError::LayoutVersion(self.layout_version));
        }
        let vk = VerifyingKey::from_bytes(reviewer_pubkey)
            .map_err(|_| CommitteeError::InvalidPublicKey)?;
        let sig = Signature::from_bytes(&self.signature);
        vk.verify_strict(&self.signing_message(), &sig)
            .map_err(|_| CommitteeError::InvalidSignature)
    }

    /// True if this record was made by `reviewer_id` on `submission_id` with `comment`.
    pub fn matches(&self, reviewer_id: &str, submission_id: &str, comment: &str) -> bool {
        self.reviewer_hash == committee_reviewer_hash(reviewer_id)
            && self.submission_hash == committee_submission_hash(submission_id)
            && self.comment_hash == committee_comment_hash(comment)
    }
}

/// Checks the append-only hash chain of a decision log.
pub fn verify_committee_chain(records: &[CommitteeDecisionRecord]) -> Result<(), CommitteeError> {
    let mut prev = [0u8; 32];
    for (i, r) in records.iter().enumerate() {
        if r.prev_hash != prev {
            return Err(CommitteeError::ChainBroken(i));
        }
        prev = r.record_hash();
    }
    Ok(())
}

/// Full verification: hash chain plus every signature. `key_for(i, rec)` returns the
/// reviewer public key the application expects for record `i` (keys come from the app/vault).
pub fn verify_committee_log<F>(
    records: &[CommitteeDecisionRecord],
    mut key_for: F,
) -> Result<(), CommitteeError>
where
    F: FnMut(usize, &CommitteeDecisionRecord) -> Option<[u8; 32]>,
{
    verify_committee_chain(records)?;
    for (i, r) in records.iter().enumerate() {
        let pk = key_for(i, r).ok_or(CommitteeError::InvalidPublicKey)?;
        r.verify_signature(&pk)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key(seed: u8) -> SigningKey {
        // Test-only deterministic key; production keys live in the app's vault.
        SigningKey::from_bytes(&[seed; 32])
    }

    fn signed(sk: &SigningKey, verdict: u32, prev: [u8; 32]) -> CommitteeDecisionRecord {
        let mut r = CommitteeDecisionRecord::new_unsigned(
            verdict,
            "prof-a",
            "sub-1",
            "ok",
            1_700_000_000,
            prev,
        )
        .unwrap();
        r.signature = sk.sign(&r.signing_message()).to_bytes();
        r
    }

    #[test]
    fn size_is_160() {
        assert_eq!(core::mem::size_of::<CommitteeDecisionRecord>(), 160);
    }

    #[test]
    fn rejects_bad_inputs() {
        for v in [0u32, 4, 99] {
            assert_eq!(
                CommitteeDecisionRecord::new_unsigned(v, "r", "s", "", 1, [0; 32]),
                Err(CommitteeError::InvalidVerdict(v))
            );
        }
        assert_eq!(
            CommitteeDecisionRecord::new_unsigned(1, " ", "s", "", 1, [0; 32]),
            Err(CommitteeError::EmptyReviewerId)
        );
        assert_eq!(
            CommitteeDecisionRecord::new_unsigned(1, "r", "", "", 1, [0; 32]),
            Err(CommitteeError::EmptySubmissionId)
        );
        let long = "x".repeat(COMMITTEE_COMMENT_MAX_BYTES + 1);
        assert!(matches!(
            CommitteeDecisionRecord::new_unsigned(1, "r", "s", &long, 1, [0; 32]),
            Err(CommitteeError::CommentTooLong(_))
        ));
    }

    #[test]
    fn sign_verify_and_tamper() {
        let sk = key(7);
        let pk = sk.verifying_key().to_bytes();
        let a = signed(&sk, 1, [0; 32]);
        let b = signed(&sk, 3, a.record_hash());
        let log = vec![a, b];
        verify_committee_log(&log, |_, _| Some(pk)).unwrap();
        assert!(a.matches("prof-a", "sub-1", "ok"));
        assert!(!a.matches("prof-a", "sub-1", "changed"));

        // verdict byte flip (Approve -> Revise) breaks signature
        let mut t = a;
        t.verdict = 2;
        assert_eq!(
            t.verify_signature(&pk),
            Err(CommitteeError::InvalidSignature)
        );
        // flip to an invalid verdict
        t.verdict = 0;
        assert_eq!(
            t.verify_signature(&pk),
            Err(CommitteeError::InvalidVerdict(0))
        );

        // chain break
        let mut broken = log.clone();
        broken[0].comment_hash[0] ^= 1;
        assert_eq!(
            verify_committee_chain(&broken),
            Err(CommitteeError::ChainBroken(1))
        );
        let mut reordered = log.clone();
        reordered.swap(0, 1);
        assert_eq!(
            verify_committee_chain(&reordered),
            Err(CommitteeError::ChainBroken(0))
        );

        // wrong key
        let other = key(8).verifying_key().to_bytes();
        assert_eq!(
            a.verify_signature(&other),
            Err(CommitteeError::InvalidSignature)
        );
    }
}
