# Design note: running an academic research lab on S3A

Status: **scenario and gap analysis, not a commitment.** Labels used:
- **[exists]**: code is in this repo (path given).
- **[outside repo]**: part of Ashley's local lab setup and not checkable from this repo (the s3a-lrs VM, `scholar.s3a` and its audit file, the Aura gateway / local AI, Orange).
- **[proposed]**: not built.

## 1. Components

| Piece | Role in the lab | Status |
|---|---|---|
| S3A archive + `s3a-cli seal` / `verify` | Canonical study store; sealed snapshots for reproducibility | [exists] `crates/s3a-cli`, `crates/s3a-engine` |
| Scholar pack (`s3a-pack-scholar`) | Papers, evidence, xAPI activity; `recheck_archive` rechecks citation fingerprints | [exists] `crates/s3a-pack-scholar` |
| Consent purposes | `Research`, `Analytics`, `ModelTraining`, `Sharing` tags on participant data | [exists] `crates/s3a-core/src/consent.rs` |
| Erasure receipts | Verifiable record that data was destroyed (GDPR Art. 17 / IRB) | [exists] `crates/s3a-core/src/erasure.rs` |
| Vault identities, `gate://` addresses, delegation grants | Who people and agents are, and what they may do | [exists] `crates/s3a-vault` (`create_delegation_grant`, `evaluate_agent_action`, `is_judgment_action`) |
| Harness with human gates | AI runs that pause for people at judgement points | [exists] `crates/s3a-harness` (`request_human_gate`, `record_human_decision_with_gate`) |
| Task tracing | Human vs AI provenance per step | [exists] `crates/s3a-trace` |
| s3a-gate (agentic GitHub) | Signed commits, supervisor authorization, review of agent work | [exists] `cloudflare/` |
| VCS (commit / branch / merge / git export) | Version history of the study archive | [exists] `crates/s3a-vcs` |
| s3a-lrs VM, `scholar.s3a` + audit file | The lab's live store | [outside repo] |
| Aura gateway / local AI | On-premises model for agents | [outside repo] |
| Orange | Interim analysis tool | [outside repo]; no bridge in repo |

## 2. Roles

- **PI**: owns the study vault. Issues delegation grants and signs off protocol, analysis and publication.
- **Researchers / students**: have their own vault identities and receive scoped grants from the PI (for example, read de-identified data, or write analysis branches). Students' work merges only after review.
- **Participants**: give consent recorded under the `consent.rs` purposes. They can withdraw, which produces an erasure receipt.
- **AI agents**: have their own identities under grants. They can search, check that a record exists, draft and run analyses. Under `is_judgment_action` they cannot approve, judge or sign off.
- **IRB / reviewer**: [proposed] a read-only grant to the audit trail, consent records and sealed snapshots, with no access to raw participant data.

## 3. Study lifecycle

1. **Protocol.** The PI commits the protocol to the study archive and it is sealed. Amendments are new commits, so history is never rewritten. *[exists: VCS + seal; proposed: a protocol record type and IRB approval record]*
2. **Consent.** Each participant's consent purposes are recorded before any of their data is ingested. *[exists: purpose enum; proposed: an ingest gate that refuses data without matching consent]*
3. **Data intake.** Data is ingested into Scholar tiles under the participant's pseudonymous ID. Participant tracing (linking a person across sessions or sources) happens **only when that participant has consented to it**. *[exists: Scholar ingest; proposed: a consent check inside ingest, plus a tracing-consent purpose, which is not in the current enum]*
4. **Analysis.** Agents and students work on branches, starting from a sealed snapshot. Harness runs record provenance (`s3a-trace`). Any judgement, such as excluding outliers or interpreting results, raises a human gate. *[exists]*
5. **Sign-off.** The PI (and IRB where required) records a signed human decision on the analysis branch. Only then is it merged. *[exists: human gates, signed commits in s3a-gate; proposed: a sign-off record type that is tied to a snapshot hash]*
6. **Publication / archive.** A final sealed snapshot is made. `verify` and `recheck_archive` must pass. The snapshot hash is cited in the paper. *[exists: seal, verify, recheck; proposed: a citation/DOI record]*

## 4. Rules the system enforces

- **AI may check that a record exists; judgements go to humans.** This is `is_judgment_action` plus human gates. [exists]
- **Consent before tracing.** [proposed] It is not enforced at ingest today.
- **Agents never write S3A core.** This is enforced by CODEOWNERS and PR review. [exists as process]

## 5. Provenance and reproducibility

- Every analysis cites the sealed snapshot it ran on. Anyone with the snapshot can rerun `s3a-cli verify` (exit 0 = OK) and `recheck_archive`. [exists]
- Steps are recorded with human vs AI provenance (`s3a-trace`). Commits are signed (s3a-vcs / s3a-gate). [exists]
- [proposed] A "reproduce" command that takes a snapshot hash plus an analysis script and checks that the output hash matches.

## 6. Data access

- Access uses vault delegation grants, scoped by action and checked by `evaluate_agent_action`. The rule against granting more than you hold is enforced in `delegation.rs`. [exists]
- Sharing an item with a collaborator uses signed `gate://` share links and QR codes (`s3a-vault share`/`open`). [exists]
- [proposed] Grant scopes tied to consent purposes. For example, a `ModelTraining` grant only reaches records whose participants consented to `ModelTraining`.

## 7. Offline / air-gapped operation

- Archives, sealing, verification, vault keys and backups (`create_backup` / `restore_backup`) are local files that need no network. [exists]
- The gate directory has offline relay mailboxes. The edge profile covers local transports (`docs/spec/S3A-EDGE-PROFILE.md`). [exists in spec/code]
- Local AI would run through the Aura gateway. [outside repo] s3a-gate currently runs on Cloudflare, so it is not air-gapped. [proposed: a self-hosted gate]

## 8. Analysis tools

- **Now:** Orange, fed by an export from S3A. [outside repo; proposed: a documented CSV/Parquet export that also writes a manifest recording the snapshot hash]
- **Later:** a free SPSS-like stats app that reads S3A directly. Every run would be a traced, reproducible step. [proposed]

## 9. Gaps, ranked by what to build first

1. **Consent-gated ingest and tracing.** Refuse ingest or linking without a matching consent purpose, and add a tracing purpose. This is the biggest ethics risk, so it comes first.
2. **Sign-off record tied to a snapshot hash.** A signed PI/IRB approval that names exactly what was approved, and that `verify` can check.
3. **Reviewer (IRB) read-only role and consent-scoped grants.** Audit without access to raw data.
4. **Analysis export with manifest (Orange bridge)** and a `reproduce` check.
5. **Protocol / amendment record types.**
6. **Self-hosted s3a-gate** for fully air-gapped labs.
7. **SPSS-like stats app** on S3A data.
