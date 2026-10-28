//! Read-only audit of the signed records in a vault.
//!
//! [`crate::Vault::open`] loads records after a CRC (and, for grants, a time-order) check, but does
//! not check signatures, so opening never fails on a forged grant or transfer. `Vault::open`
//! keeps that behavior; call [`crate::Vault::verify_records`] to learn which signed records hold up.
//! Nothing here writes, and nothing here is a judgement: a failed check is reported, not acted on.

use std::collections::HashMap;

use crate::delegation::verify_delegation_grant;
use crate::records::{DelegationGrantRecord, ItemTransferRecord, VaultItemRecord};
use crate::transfer::{resolve_chain_ownership, verify_transfer_record};

/// Signature check of one delegation grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantCheck {
    pub grant_id: [u8; 16],
    pub granter_pubkey: [u8; 32],
    pub agent_pubkey: [u8; 32],
    pub ok: bool,
    /// Why it failed, in the vault's own words; `None` when `ok`.
    pub error: Option<String>,
}

/// Signature and CRC check of one transfer record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferCheck {
    pub transfer_id: [u8; 16],
    pub item_hash: [u8; 32],
    /// Position in the item's chain, in file order (0 = genesis link).
    pub index: usize,
    pub from_pubkey: [u8; 32],
    pub to_pubkey: [u8; 32],
    pub ok: bool,
    pub error: Option<String>,
}

/// Ownership-chain check for one item (hash links, signer-is-current-owner, double-send rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainCheck {
    pub item_hash: [u8; 32],
    pub links: usize,
    pub ok: bool,
    /// Current owner key when the chain resolves.
    pub owner_pubkey: Option<[u8; 32]>,
    pub error: Option<String>,
}

/// Result of [`crate::Vault::verify_records`]. Lists are sorted (by id / item hash / position) so
/// the output is deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VaultVerifyReport {
    pub grants: Vec<GrantCheck>,
    pub transfers: Vec<TransferCheck>,
    pub chains: Vec<ChainCheck>,
}

impl VaultVerifyReport {
    /// Every grant signature, transfer signature and ownership chain held up.
    pub fn all_ok(&self) -> bool {
        self.grants.iter().all(|g| g.ok)
            && self.transfers.iter().all(|t| t.ok)
            && self.chains.iter().all(|c| c.ok)
    }
}

pub(crate) fn verify_all(
    grants: &HashMap<[u8; 16], DelegationGrantRecord>,
    items: &HashMap<[u8; 32], VaultItemRecord>,
    transfers: &HashMap<[u8; 32], Vec<ItemTransferRecord>>,
) -> VaultVerifyReport {
    let mut report = VaultVerifyReport::default();

    let mut gids: Vec<_> = grants.keys().copied().collect();
    gids.sort();
    for id in gids {
        let g = &grants[&id];
        let r = verify_delegation_grant(g);
        report.grants.push(GrantCheck {
            grant_id: g.grant_id,
            granter_pubkey: g.granter_pubkey,
            agent_pubkey: g.agent_pubkey,
            ok: r.is_ok(),
            error: r.err().map(|e| e.to_string()),
        });
    }

    let mut hashes: Vec<_> = transfers.keys().copied().collect();
    hashes.sort();
    for h in hashes {
        let chain = &transfers[&h];
        for (index, t) in chain.iter().enumerate() {
            let r = verify_transfer_record(t);
            report.transfers.push(TransferCheck {
                transfer_id: t.transfer_id,
                item_hash: h,
                index,
                from_pubkey: t.from_pubkey,
                to_pubkey: t.to_pubkey,
                ok: r.is_ok(),
                error: r.err().map(|e| e.to_string()),
            });
        }
        // The same issuer rule the vault applies when it records a transfer.
        let issuer = items
            .get(&h)
            .map(|i| i.issuer_pubkey)
            .unwrap_or_else(|| chain[0].from_pubkey);
        let r = resolve_chain_ownership(&issuer, chain);
        report.chains.push(ChainCheck {
            item_hash: h,
            links: chain.len(),
            ok: r.is_ok(),
            owner_pubkey: r.as_ref().ok().copied(),
            error: r.err().map(|e| e.to_string()),
        });
    }
    report
}
