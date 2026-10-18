# S3A Formal Research & Agent Role Ontologies

This directory contains the formal OWL 2 DL ontologies, xAPI profiles (W3C/ADL conforming to xAPI Profiles v1.0), and resource manifests for the Scholar Research Traceability system and per-agent autonomous roles in S3A Gate.

---

## Role-to-Resource Mapping Table

| Role | Actor Class | xAPI Profile | OWL 2 DL Ontology | Resource Manifest | Allowed Profile Verbs | Core Prohibitions | Output Schema |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Base Scholar** | `:HumanActor` / `:AIAgent` | [`scholar-xapi-profile.jsonld`](scholar-xapi-profile.jsonld) | [`scholar.ttl`](scholar.ttl) | — | Base scientific research & review verbs | Disjointness of HumanActor & AIAgent | Scientific Claim / Citation |
| **`pm`** | `:PMAgent` | [`pm-xapi-profile.jsonld`](pm-xapi-profile.jsonld) | [`pm.ttl`](pm.ttl) | [`pm-manifest.json`](pm-manifest.json) | `planned`, `assigned`, `prioritized`, `scheduled`, `requested-human-review`, `experienced` | No judgment verbs; no code patches | `PMExecutionPlan` (`steps`, `files_allowed`, `tests`) |
| **`builder`** | `:BuilderAgent` | [`builder-xapi-profile.jsonld`](builder-xapi-profile.jsonld) | [`builder.ttl`](builder.ttl) | [`builder-manifest.json`](builder-manifest.json) | `edited`, `authored-diff`, `built`, `tested`, `committed`, `submitted-change-request`, `requested-human-review` | No judgment verbs; no unscoped edits; no direct main merge | `BuilderPatchOutput` (`diff`, `touched_files`, `commit_sha`) |
| **`qa`** | `:QAAgent` | [`qa-xapi-profile.jsonld`](qa-xapi-profile.jsonld) | [`qa.ttl`](qa.ttl) | [`qa-manifest.json`](qa-manifest.json) | `tested`, `verified-claim`, `flagged-discrepancy`, `captured-screenshot`, `exported-snapshot`, `requested-human-review` | Read & test only; no code patches; no commits | `QAEvidenceReport` (`run_id`, `evidence_verified`, `screenshots`) |
| **`budget`** | `:BudgetAgent` | [`budget-xapi-profile.jsonld`](budget-xapi-profile.jsonld) | [`budget.ttl`](budget.ttl) | [`budget-manifest.json`](budget-manifest.json) | `metered`, `audited-ledger`, `deferred-action`, `reconciled`, `flagged-discrepancy` | Money agent counts only; no financial transfers; no patches | `LedgerEntryOutput` (`kind`, `units`, `est_cost`, `cumulative`) |
| **`librarian`** | `:LibrarianAgent` | [`librarian-xapi-profile.jsonld`](librarian-xapi-profile.jsonld) | [`librarian.ttl`](librarian.ttl) | [`librarian-manifest.json`](librarian-manifest.json) | `indexed`, `searched`, `retrieved`, `cited`, `exported-snapshot`, `flagged-discrepancy` | No production code patches; no judgment verbs | `LibrarianKnowledgeArtifact` (`doi`, `quote`, `sha256`) |
| **`scrum`** | `:ScrumAgent` | [`scrum-xapi-profile.jsonld`](scrum-xapi-profile.jsonld) | [`scrum.ttl`](scrum.ttl) | [`scrum-manifest.json`](scrum-manifest.json) | `facilitated-standup`, `reported-blocker`, `synchronized-state`, `summarized`, `requested-human-review` | Swarm coordination only; no code patches | `ScrumStandupReport` (`active_agents`, `blocked_tasks`, `conflicts`) |

---

## Architectural & Ontological Invariants

1. **Strict Disjointness Axiom:**
   ```turtle
   :HumanActor owl:disjointWith :AIAgent .
   ```
   No AI model, agent loop, or autonomous process may act as a `:HumanActor`.
2. **Judgment Exclusivity:**
   ```turtle
   :Judgment a owl:Class ;
       owl:onProperty prov:wasAssociatedWith ;
       owl:allValuesFrom :HumanActor .
   ```
   Authoritative evaluative decisions (e.g. `approved`, `rejected`, environment promotions, baseline merges) require human moral agency and supervisory authority (Ashley or Ward signed Ed25519 keys).
3. **Role Scope Constraints:**
   - **Review / QA Roles:** Strictly read-only (`files_allowed: []` or `role: "qa"`). Gate rejects any trace statement producing code diffs or patches.
   - **Budget Role:** Compute unit metering only. The budget agent records LLM token counts, sandbox seconds, browser minutes, and R2 operations, but never touches financial cashier/escrow/payout primitives.
   - **Builder Role:** Code generation is strictly bounded to the assigned task's `files_allowed` array. Changes outside scope or touching sensitive modules (`cashier`, `escrow`, `payout`, `auth`, `db`, `infra`) require human draft escalation.

---

## API Endpoints

The Gate exposes all vocabulary packs and manifests via REST:
- `GET /project/:projectId/vocab/profile`: Base Scholar xAPI Profile (JSON-LD)
- `GET /project/:projectId/vocab/ontology`: Base Scholar Ontology (Turtle TTL)
- `GET /project/:projectId/vocab/roles`: List all 6 role manifests, profiles, and ontologies
- `GET /project/:projectId/vocab/roles/:role`: Get specific role manifest, profile (`?format=jsonld`), or ontology (`?format=ttl`)
- `GET /project/:projectId/vocab/resolve?iri=<iri_or_verb>`: Ontological resolution & classification check
