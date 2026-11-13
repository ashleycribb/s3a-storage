# S3A Sovereign Agentic VCS & Code Platform (Cloudflare Edge)

* **Author:** S3A Architecture Team
* **Status:** Proposed
* **Date:** 2026-10-31

--------------------------------------------------------------------------------

## Overview

The S3A Sovereign Agentic Platform is an edge-native, AI-first alternative to GitHub designed specifically for autonomous agent swarms, human supervisors, and verifiable code development. Built entirely on Cloudflare edge primitives (Workers, Durable Objects, `@cloudflare/sandbox`, R2, and Workers AI), the platform replaces traditional Git hosting, issues, and pull requests with a cryptographically verifiable **Change Request (CR)** engine, interactive human-PM alignment, and formal **W3C/ADL xAPI tracing** grounded in **OWL 2 DL ontologies**.

The platform is designed around a collaborative hierarchy of specialized agents:
1. **`pm` (Project Coordinator):** Conducts interactive intake interviews with the human owner/developer to scope problems, establish boundaries, and construct a formal Plan of Attack.
2. **`gate-agent` (Senior Dev / Builder):** Translates requirements into scoped code patches within isolated micro-sandboxes, ensuring all tests pass green.
3. **`antigravity` (Junior Dev / Reviewer):** Audits architecture, flags discrepancies with exact `file:line` citations, and evaluates design layouts via the live Vector Visual Review Canvas.
4. **`budget` (Financial Governor):** Enforces hard daily, weekly, and monthly dollar and token spend caps on Cloudflare Workers AI and compute, using deterministic SQLite ledgers to pause workloads and emit deduplicated notifications.
5. **`librarian` (Self-Improvement Engine):** Continuously indexes execution traces, failure fingerprints, and verified repairs into S3A knowledge tiles, enabling the swarm to learn and improve diagnostic accuracy over time.

To preserve absolute safety, the platform enforces the formal ontological invariant that AI agents are disjoint from human actors (`:HumanActor owl:disjointWith :AIAgent`). All functional and performance-impacting code changes require cryptographic human supervisor signatures (Ed25519) before merging into baseline branches, while benign non-functional updates (e.g. pure documentation or visual tokens) may auto-merge under strict glob-based path policies.

## The Problem

Traditional source code platforms (such as GitHub, GitLab, and Bitbucket) were architected for human-centric workflows, manual code review, and asynchronous human communication. When applied to high-concurrency autonomous agent swarms, these platforms break down:

1. **Lack of Agent Governance & Financial Bounds:** Standard Git platforms have no awareness of LLM token economics or compute budgets. Unmonitored recursive agent loops burn through API limits and cloud hosting quotas without centralized throttling.
2. **Missing Semantic Traceability & Hallucination Drift:** Conventional Git commits and PR comments lack machine-readable semantic provenance. There is no formal specification governing what actions an agent may perform, allowing QA bots to author unvetted code patches or agents to approve their own pull requests.
3. **Absence of Continuous Platform Self-Improvement:** Git platforms treat closed PRs and issue comments as dead unstructured text. An agent that solves a subtle race condition in one task cannot query structured semantic memory to solve similar defects across other repositories or future tasks.
4. **Serverless Boundary Mismatch:** Edge serverless architectures lack the ability to run POSIX compilers (`cargo`, `npm`) directly within V8 isolates, requiring a dedicated, isolated execution sandbox layer.

The S3A Sovereign Platform solves these challenges by combining edge-native Durable Object state coordination, strict sandbox verification, formal OWL/xAPI governance invariants, and a closed-loop semantic learning engine.

## Goals

* **Sovereign Agentic VCS & Change Requests:** Provide a full-featured alternative to GitHub Issues, Pull Requests, and Git repositories powered by Cloudflare Durable Objects and S3A VCS containers.
* **Formal xAPI Event Bus & Zero Vocabulary Drift:** Record every agent and human action as a verifiable xAPI activity statement grounded 100% in formal JSON-LD profiles and OWL 2 DL ontologies (`pm`, `builder`, `qa`, `budget`, `librarian`, `scrum`).
* **Ontological Invariant Enforcement:** Enforce at the edge runtime that AI agents cannot emit human supervisory judgment verbs (`approved`, `rejected`) and that non-builder roles cannot author code diffs.
* **Continuous Swarm Self-Improvement:** Index verified error fingerprints, test reproduction traces, and successful code diffs into S3A knowledge containers via the Librarian Agent, enabling cross-task learning.
* **Deterministic Financial Governance:** Track and enforce daily, weekly, and monthly dollar caps on Cloudflare Workers AI and LLM inference spend via zero-token Durable Object SQLite accounting.
* **Isolated Sandbox Build & Verification:** Ensure every proposed code patch compiles and passes comprehensive test suites green within an isolated `@cloudflare/sandbox` container before being submitted for review.
* **Visual Observability Canvas:** Render interactive SVG pipeline graphs reflecting task progression, AST component hierarchies, visual design wireframes, and live log streams.
* **Role-Based Merge Safeguards:** Enforce mandatory human approval for all functional and performance-impacting code changes, while permitting policy-defined auto-merge only for benign non-functional updates verified by path glob matching (`docs/**`, `*.md`).

## Non-Goals

* **Unattended Direct Production Deploys:** The platform will never push code changes directly to live production environments; all changes are submitted as repository Change Requests.
* **Arbitrary Native Code Execution inside Workers Isolates:** Workers do not attempt to run compilers or language runtimes locally; all build and test execution is delegated to container sandboxes.
* **Unrestricted Auto-Merging:** Functional code changes, architecture modifications, or security-sensitive modules (`escrow`, `auth`, `payout`, `infra`) will never bypass human supervisor review.

## Critical User Journeys (CUJs)

### CUJ 1: Human Interview, Swarm Diagnosis, Sandbox Repair, and Monitored Merge

* **Given:** A repository owner reports an issue with tournament matchmaking in an S3A-hosted project.
* **When:** The owner opens the platform web interface, completes an interactive intake interview with the PM agent (`xapi:planned`), and kicks off the repair.
* **Then:** 
  1. The PM agent scopes the task and assigns `files_allowed: ["lib/matchmaking.ts"]`.
  2. The Senior Dev (`gate-agent`) synthesizes a patch (`xapi:authored-diff`) and dispatches a container sandbox run (`xapi:tested`).
  3. The sandbox runs `npm ci` and `npm test`, verifying the build exits 0.
  4. The Junior Dev (`antigravity`) reviews the code diff, verifies AST layout on the Visual Review Canvas, and confirms zero regressions.
  5. The owner reviews the Change Request in the web console, and signs off with an Ed25519 supervisor key (`xapi:approved`), merging the CR into baseline.
  6. The Librarian Agent indexes the error fingerprint and verified fix into S3A knowledge tiles (`xapi:indexed`).

### CUJ 2: Budget Cap Exhaustion, Deduplicated Notification, and Resumption

* **Given:** A burst of machine-reported defects consumes the daily Cloudflare Workers AI dollar budget ($5.00 cap).
* **When:** A new incident is ingested.
* **Then:** The `BudgetCoordinatorDO` detects that available funds are depleted; the task is placed into the priority backlog; exactly one deduplicated notification is dispatched to the designated channel (Webhook, Email, or O365); and an internal SQLite timer queue schedules a Durable Object alarm to wake the queue at `00:00:01 UTC` to resume execution without manual intervention.

--------------------------------------------------------------------------------

## Technical Plan (Architecture)

### High-Level System Architecture

The platform architecture is structured into four distinct, cooperating tiers:

```mermaid
flowchart TD
    subgraph Human["Human Supervisor"]
        User["Human Owner / Developer"]
        UI["Web Console & SVG Review Canvas\n(Live SSE Stream, Diff Viewer)"]
    end

    subgraph Gateway["Edge Gateway & Ontological Enforcement (Workers)"]
        Worker["Cloudflare Worker Router"]
        Vocab["xAPI & OWL 2 DL Validator\n(/vocab/profile, /vocab/resolve)"]
        SSE["SSE Live Activity Stream"]
    end

    subgraph State["Deterministic State & Governance (Durable Objects)"]
        BudgetDO["BudgetCoordinatorDO\n(SQLite Token/Dollar Ledger, Alarm Queue)"]
        RepoDO["RepoCoordinatorDO\n(CR Engine, Priority Queue, Task Lifecycle)"]
    end

    subgraph Swarm["Autonomous Agent Swarm (Workers AI / Gateway)"]
        PM["PM Agent\n(Task Scoping & Interview)"]
        Senior["Senior Dev (gate-agent)\n(Patch Synthesis)"]
        Junior["Junior Dev (antigravity)\n(Code & AST Review)"]
        Librarian["Librarian Agent\n(Knowledge Indexing & Memory)"]
    end

    subgraph Execution["Sandboxed Execution & Provenance"]
        Sandbox["@cloudflare/sandbox Container\n(npm ci, npm test, Anti-Tamper)"]
        R2["Cloudflare R2\n(Forensic Bundles, S3A Snapshot Blobs)"]
        WASM["s3a-wasm-snapshot\n(In-Memory Tile Delta Sync)"]
    end

    subgraph VCS["S3A Sovereign VCS"]
        CR["Change Request Engine\n(Ed25519 Human Approval Barrier)"]
    end

    User <-->|Interactive Intake Interview| PM
    User <-->|Observe Execution & Review| UI
    Worker --> SSE --> UI
    Worker --> Vocab
    Worker <--> RepoDO
    RepoDO <-->|Debit / Reserve Budget| BudgetDO
    RepoDO -->|Dispatch Scoped Task| Senior
    Senior -->|Submit Candidate Patch| Sandbox
    Junior -->|Audit Diff & Wireframe| UI
    Sandbox -->|Verify Green & Anti-Tamper| Sandbox
    Sandbox -->|Stream Logs & Diffs| R2
    Sandbox -->|Commit S3A Artifacts| WASM
    Sandbox -->|Execution Passed (Exit 0)| RepoDO
    RepoDO -->|Policy Guard: Benign vs Functional| CR
    User -->|Sign-Off (Ashley/Ward Ed25519)| CR
    CR -->|Index Verified Fix| Librarian
    Librarian -->|Persist Pattern| R2
```

### Component Breakdown

#### 1. Deterministic State Governance (`RepoCoordinatorDO` & `BudgetCoordinatorDO`)
* **`RepoCoordinatorDO` (Single Atomic Repository Authority):** Consolidates the Change Request engine, issue state machine (`PENDING` -> `SANDBOX_BUILD` -> `VERIFYING` -> `NEEDS_JUDGMENT` -> `MERGED`), and the prioritized task backlog (Safety Critical > P0 > FIFO). Co-locating queue and repository state within a single SQLite-backed Durable Object eliminates distributed transactions, cross-DO RPC latency, and deadlock risks.
* **`BudgetCoordinatorDO` (Tenant Financial Ledger):** Tracks token counts and dollar spend against daily, weekly, and monthly caps. Implements a multi-timer queue inside SQLite that sets the single native DO `alarm()` for the earliest pending deadline, avoiding alarm collisions between daily rollovers and task timeouts.

#### 2. Formal xAPI Event Bus & OWL 2 DL Ontology Enforcement
* Every agent action emits an xAPI statement conforming to W3C/ADL standards:
  * `:PMAgent` emits `planned`, `assigned`, `prioritized`, `scheduled`.
  * `:BuilderAgent` emits `edited`, `authored-diff`, `built`, `tested`, `committed`, `submitted-change-request`.
  * `:QAAgent` emits `tested`, `verified-claim`, `flagged-discrepancy`, `captured-screenshot`.
  * `:BudgetAgent` emits `metered`, `audited-ledger`, `deferred-action`.
  * `:LibrarianAgent` emits `indexed`, `searched`, `retrieved`, `cited`.
* **Runtime Gate Invariant:** The Worker router runs `/vocab/resolve` validation. Any agent attempting to emit a judgment verb (`approved`, `rejected`) or unpermitted action is blocked with HTTP 403 `ONTOLOGY_VIOLATION_AI_JUDGMENT_FORBIDDEN`.

#### 3. Ephemeral Container Sandbox (`@cloudflare/sandbox`)
* Executes builds and tests inside isolated container environments using `@cloudflare/sandbox`.
* **Test Anti-Tampering Hook:** Inspects git diffs on test suites. Completely forbids deleting or weakening existing test assertions, while permitting the addition of new reproduction test cases.
* **Credential Isolation:** The container operates in an isolated network namespace with zero ambient access to Cloudflare API tokens or repository push keys.

#### 4. S3A Storage & In-Memory WASM Bridge (`s3a-wasm-snapshot`)
* Leverages `crates/s3a-wasm-snapshot` compiled to WebAssembly to unpack, inspect, and delta-sync `.s3a` snapshot containers directly in Worker memory without filesystem I/O.
* **Tile Schema Alignment:**
  * Tile 10 (`HumanLrsRecord`) & Tile 19 (`MarkRecord`): Record administrative status marks (`MARK_APPROVED`, `MARK_NEEDS_REVIEW`).
  * Tile 70 (`SEAL`) & Tile 80 (`VCS_COMMIT`): Store cryptographic Ed25519 supervisor signatures (`commit_sig_hex`) and Merkle tree root hashes.
  * Tile 72 (`PROVENANCE`): Stores machine-readable xAPI activity statements.

#### 5. Role-Based Merge Safeguards (`MergePolicyGuard`)
* **Human Approval Barrier:** Functional bug fixes, performance modifications, and system logic changes generate an immutable Change Request requiring an authorized supervisor signature (Ashley or Ward Ed25519 key).
* **Deterministic Auto-Merge Fast Path:** Restricted strictly to non-functional changes verified by file path glob matching (`docs/**`, `*.md`, `brand/tokens.json`) combined with `git diff --stat` confirming 0 lines changed in functional code.

--------------------------------------------------------------------------------

## Alternatives Considered

### Alternative A: Using GitHub as the Underlying Platform with Bot Webhooks

* **Pros:** Leverages existing GitHub UI, user accounts, and infrastructure.
* **Cons:** Cannot enforce edge-native financial spending bounds; no native support for W3C xAPI activity tracing or OWL 2 DL ontological invariants; exposes development workflows to third-party API rate limits.
* **Reason for Rejection:** The system is explicitly intended as a sovereign, self-improving platform built on Cloudflare and S3A that natively replaces GitHub for agent swarms.

### Alternative B: Conversational LLMs for Budget & Queueing

* **Pros:** Rapid prompt-based prototyping.
* **Cons:** Severe token waste counting tokens; stochastic hallucinations in financial ledgers; high latency.
* **Reason for Rejection:** Accounting and priority queues are deterministic mathematical operations. Replaced with SQLite ledgers inside Durable Objects.

### Alternative C: Multi-Language AST Diff Parsing in Cloudflare Workers

* **Pros:** Deep structural code understanding.
* **Cons:** Severe memory and CPU overhead inside 128 MB V8 isolates; requires bundling multi-language parsers.
* **Reason for Rejection:** Over-engineered for merge protection. Path globbing (`docs/**`) and diff stat verification provide 100% safety with zero runtime overhead.

### Alternative D: Bespoke External MicroVM Sandbox Infrastructure

* **Pros:** Total control over VM kernel parameters.
* **Cons:** Requires managing external VM fleets; introduces complex credential exfiltration risks.
* **Reason for Rejection:** Cloudflare already provides native container sandboxing via `@cloudflare/sandbox`.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

### Target Files to Create/Modify

#### 1. `cloudflare/wrangler.toml` (Modify)
* **Type / Scope:** Configuration
* **Proposed Changes:** Register Durable Object bindings for `RepoCoordinatorDO` and `BudgetCoordinatorDO` under `[durable_objects].bindings`. Define migration tags for SQLite persistence.

#### 2. `cloudflare/src/index.ts` (Modify)
* **Type / Scope:** Worker Entrypoint
* **Proposed Changes:** Export `RepoCoordinatorDO` and `BudgetCoordinatorDO` classes. Route REST endpoints for `/vocab/profile`, `/vocab/ontology`, `/vocab/resolve`, `/project/:id/change-requests`, and `/control`.

#### 3. `cloudflare/package.json` (Modify)
* **Type / Scope:** Package Manifest
* **Proposed Changes:** Add `"typecheck": "tsc --noEmit"`, install `vitest` and `@cloudflare/vitest-pool-workers` as devDependencies, and add `"test": "vitest run"`.

#### 4. `cloudflare/src/do/RepoCoordinator.ts` (Modify)
* **Type / Scope:** TypeScript Durable Object
* **Proposed Changes:** Consolidate priority queue backlog, Change Request lifecycle (`PENDING` -> `VERIFYING` -> `NEEDS_JUDGMENT` -> `MERGED`), and task scoping within the existing unified `RepoCoordinatorDO`.

#### 5. `cloudflare/src/do/BudgetCoordinatorDO.ts` (Create)
* **Type / Scope:** TypeScript Durable Object
* **Proposed Changes:** Implement SQLite-backed token and dollar expenditure tracking with support for daily, weekly, and monthly balance limits. Implements multi-timer queue handling to schedule the single native DO `alarm()` for earliest pending deadline.

#### 6. `cloudflare/src/agents/PmInterviewAgent.ts` (Create)
* **Type / Scope:** TypeScript / Workers AI Service
* **Proposed Changes:** Coordinates conversational intake interview with human users. Generates structured `PMExecutionPlan` (`steps`, `files_allowed`, `tests`) conforming to the PM xAPI profile.

#### 7. `cloudflare/src/sandbox/SandboxRunner.ts` (Create)
* **Type / Scope:** TypeScript Integration Client
* **Proposed Changes:** Wraps `@cloudflare/sandbox` container runtime. Checks out pinned commits, installs dependencies via `npm ci`, runs tests, enforces test anti-tampering rules (no assertion deletions), and streams logs to Cloudflare R2.

#### 8. `cloudflare/src/wasm/S3aWasmBridge.ts` (Create)
* **Type / Scope:** TypeScript WebAssembly Wrapper
* **Proposed Changes:** Integrates `s3a-wasm-snapshot` WebAssembly module. Encapsulates agent execution traces into Tile 72 (`PROVENANCE`), administrative approval marks into Tile 19 (`MarkRecord`), and cryptographic Ed25519 signatures into Tile 80 (`VCS_COMMIT`).

#### 9. `cloudflare/src/policy/MergePolicyGuard.ts` (Create)
* **Type / Scope:** TypeScript Policy Engine
* **Proposed Changes:** Validates candidate patches using path glob matching (`docs/**`, `*.md`, `brand/tokens.json`) and diff stats. Blocks functional patches from auto-merging while permitting benign non-functional updates.

#### 10. `cloudflare/src/agents/LibrarianAgent.ts` (Create)
* **Type / Scope:** TypeScript / S3A Knowledge Service
* **Proposed Changes:** Indexes resolved error fingerprints, test reproduction traces, and verified patches into S3A knowledge containers in R2, enabling future agent runs to retrieve known repair patterns.

### Verification Plan

* **Compilation & Typecheck:**
  * In `cloudflare/`: `npm run typecheck` exits code 0.
  * In `crates/s3a-wasm-snapshot`: `wasm-pack build --target web` exits code 0.
* **Unit & Integration Tests:**
  * `cloudflare/tests/budget_coordinator.test.ts`: Validates spend reservations, threshold blocks, and multi-timer alarm scheduling.
  * `cloudflare/tests/repo_coordinator.test.ts`: Verifies priority queue ordering (Safety > P0 > FIFO) and single-alert deduplication.
  * `cloudflare/tests/sandbox_anti_tampering.test.ts`: Asserts that patches deleting existing assertions are rejected while patches adding new test cases succeed.
  * `cloudflare/tests/vocab_drift.test.ts`: Verifies that 100% of emitted verbs ground in the OWL 2 DL ontology and that AI approval attempts are rejected with HTTP 403.
* **End-to-End Simulation:**
  * `node scripts/register-antigravity-and-prove.mjs`: Proves human intake -> builder sandbox execution -> junior reviewer audit -> Ed25519 supervisor approval -> baseline merge.
