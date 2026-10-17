# Autonomous Platform Engineering Pattern: The Rehearsal-Driven Development Loop

> **Architecture Specification & Generic Engine Blueprint**  
> *S3A Platform Architecture & Autonomous Agent Swarm Verification Pattern*

---

## 1. Executive Summary & Core Thesis

Traditional software development relies on human developers writing code against ambiguous tickets, followed by manual QA cycles that leave edge cases undiscovered until production.

The **Rehearsal-Driven Autonomous Platform Pattern** replaces ticket-shuffling with a **closed-loop simulation harness**:
1. **Define the domain as data**: Topologies, bracket shapes, business rules, map pools, and game packs are declarative JSON/YAML schemas—versioned like code with cryptographic content hashes and signatures.
2. **Run scripted end-to-end simulations**: Scripted mock personas execute complete end-to-end user journeys in staging environments with zero production financial or data risk.
3. **Auto-file deduplicated issues**: Any runtime exception, missing route (`needs-page`), broken button, or unhandled rule deadlock (`needs-logic`) is trapped, snapshotted, fingerprinted, and filed into the task queue.
4. **Agents propose fixes as reviewed PRs**: Specialized AI agents claim issues, execute containerized sandbox builds, and open Pull Requests containing reproduction logs and visual evidence.
5. **Human supervisor dual-approval & green replay**: Human supervisors (Ashley & Ward) inspect PRs and cryptographically approve merges via Ed25519 signatures. The harness re-executes the exact failing simulation journey until all stages pass green.

```mermaid
flowchart TD
    D["1. Domain as Data\n(Formats, Game Packs, Rules)"] --> J["2. Scripted Journey\n(Headless Actors & APIs)"]
    J --> Check{"Journey\nPassed?"}
    Check -- Yes --> Green["All Stages Green\nProduction Ready"]
    Check -- No --> Trap["3. Deduplicated Auto-Filer\n(needs-page / needs-logic / bug)"]
    Trap --> Agent["4. Agent PR Proposal\n(Sandbox Build + Trace Diff)"]
    Agent --> Sup["Human Dual-Control\n(Ashley + Ward Ed25519)"]
    Sup --> Replay["5. Re-Run Journey Simulation"]
    Replay --> Check
```

---

## 2. The 5-Stage Autonomous Platform Pattern

### Stage 1: Define the Domain as Data (Formats / Rules / Packs)
- **Principle**: Zero hardcoded business logic or layout topologies in application code.
- **Implementation**: The application engine compiles data structures (`TournamentFormatConfig`, `GamePack`, `CarrierConfig`, `WorkflowRule`) defining:
  - Participants / Actors per session
  - Win/completion conditions (First-to-N, threshold, points, SLA)
  - Tie-breakers and rematch restrictions
  - State advancement and qualification trees
  - Configurable event options (match duration, veto pools, check-in windows, roster limits)
- **Traceability**: Every configuration file receives a deterministic SHA-256 specification hash (`computeSpecHash`) and an Ed25519 signature. New formats or game packs are proposed as pull requests—never pushed ad-hoc.

### Stage 2: Run a Scripted End-to-End Simulation / Journey
- **Principle**: If software cannot run its own happy path and edge cases autonomously, it is not ready for human users.
- **Implementation**: A generic tournament/journey runner spins up deterministic mock actors:
  - Scripted player/customer signups
  - Organizer bracket creation or order dispatch
  - Check-in, lobby assignment, and map/rule vetoes
  - Score reporting, webhook ingestion, and dispute generation
  - Advancements, playoffs, and final standings calculation
- **Safety Invariant**: Zero real money or cashier endpoints are touched. All wallets, transactions, and inventories use synthetic staging tokens.

### Stage 3: Auto-File Deduplicated Issues (`needs-page`, `needs-logic`, `bug`)
- **Principle**: Failures must be self-reporting, reproducible, and deduplicated.
- **Implementation**: When an unhandled state, missing endpoint, or broken control is hit:
  1. Capture the console error, HTTP payload, duration, and error stack trace.
  2. Capture R2 visual evidence screenshot via Browser Rendering.
  3. Generate a deterministic SHA-256 error fingerprint:
     $$\text{Fingerprint} = \text{Hash}(\text{ExceptionType} + \text{Message} + \text{SourceFile} + \text{JourneyStep})$$
  4. Query the task repository. If an open task with this fingerprint exists, increment `repro_count` and append the new run trace (avoiding ticket flood). If new, create an issue tagged with:
     - `needs-page`: Missing route, 404 handler, or disabled/broken interactive control.
     - `needs-logic`: Business rule deadlock (e.g. unhandled tie-break, score deadlock, unhandled qualification).
     - `bug`: Runtime exception, API crash, or state mismatch.

### Stage 4: Agents Propose Fixes as Reviewed Pull Requests
- **Principle**: Agents never push directly to main; agents propose auditable solutions.
- **Implementation**:
  1. Agent claims the deduplicated task via `POST /project/:id/tasks/claim`.
  2. Spawns an isolated ephemeral container sandbox (`sandbox_build`).
  3. Modifies source code, verifies typecheck (`tsc`) and automated unit tests.
  4. If the issue is tagged `needs-page`, agent outputs proposed component UI design. If tagged `needs-logic`, agent outputs proposed state transition design.
  5. Opens a Change Request (PR) with:
     - Exact Git commit SHA & sandbox image digest
     - Real build exit code, duration, and R2 log URLs (no hardcoded "Passed" text)
     - Before & after xAPI trace IDs

### Stage 5: Re-Run the Same Simulation Until Green
- **Principle**: Proof of resolution is passing the exact simulation that previously failed.
- **Implementation**:
  1. The harness re-executes the identical simulation seed and scenario against the agent's patch branch or merged preview.
  2. Compares new stage outcomes with baseline trace steps.
  3. Supervisors (Ashley & Ward) review diffs and cryptographic audit proofs before merging.
  4. Once all stages report `passed`, the application promotes to production.

---

## 3. The Generic Platform Harness (No Brakets Assumptions)

Below is the complete, domain-agnostic generic platform harness implementation. It can drive any domain (fintech escrow, logistics dispatch, tournament brackets, AI workflow routing).

```typescript
// cloudflare/src/harness/GenericPlatformHarness.ts
// Domain-Agnostic Autonomous Rehearsal & Auto-Filing Engine

export interface DomainSpec {
  id: string;
  name: string;
  version: string;
  content_hash: string;
  signature: string;
  rules: Record<string, any>;
}

export interface JourneyStageResult {
  stage_id: string;
  name: string;
  status: "passed" | "failed" | "skipped";
  duration_ms: number;
  details?: Record<string, any>;
  error?: string;
  stack?: string;
}

export interface JourneyRunRecord {
  run_id: string;
  domain_id: string;
  spec_hash: string;
  scenario: string;
  status: "passed" | "failed";
  stages: JourneyStageResult[];
  issues_filed: string[];
  total_duration_ms: number;
}

export interface TaskAutoFileRequest {
  title: string;
  repro_steps: string[];
  error_text: string;
  fingerprint: string;
  run_id: string;
  tags: Array<"needs-page" | "needs-logic" | "bug">;
  journey_step: string;
  expected_result: string;
  actual_result: string;
  screenshot_url?: string;
}

export class GenericPlatformHarness<TContext, TSpec extends DomainSpec> {
  private spec: TSpec;
  private scenario: string;
  private runId: string;
  private stages: JourneyStageResult[] = [];
  private issuesFiled: string[] = [];
  private coordinator: any;

  constructor(params: {
    spec: TSpec;
    scenario: string;
    coordinator: any;
    prefix?: string;
  }) {
    this.spec = params.spec;
    this.scenario = params.scenario;
    this.coordinator = params.coordinator;
    const p = params.prefix || "journey";
    this.runId = `${p}_${this.spec.id}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
  }

  /**
   * Deterministic SHA-256 Error Fingerprinter for deduplication.
   */
  public static computeFingerprint(errorType: string, message: string, location: string): string {
    const raw = `${errorType}:${message.trim().toLowerCase().slice(0, 100)}:${location.trim().toLowerCase()}`;
    let hash = 0;
    for (let i = 0; i < raw.length; i++) {
      hash = (hash << 5) - hash + raw.charCodeAt(i);
      hash |= 0;
    }
    return `fp_${errorType.toLowerCase().slice(0, 8)}_${Math.abs(hash).toString(16).padStart(8, "0")}`;
  }

  /**
   * Executes a lifecycle stage with automatic error trapping, timing, and failure filing.
   */
  public async executeStage(
    stageId: string,
    stageName: string,
    action: () => Promise<Record<string, any>>
  ): Promise<boolean> {
    const t0 = Date.now();
    try {
      const details = await action();
      this.stages.push({
        stage_id: stageId,
        name: stageName,
        status: "passed",
        duration_ms: Date.now() - t0,
        details,
      });
      return true;
    } catch (err: any) {
      const durationMs = Date.now() - t0;
      const isMissingPage = err.message?.includes("404") || err.name === "RouteNotFoundException";
      const isMissingLogic = err.name === "UnexpressibleRuleException" || err.message?.includes("Deadlock");
      const tag: "needs-page" | "needs-logic" | "bug" = isMissingPage
        ? "needs-page"
        : isMissingLogic
        ? "needs-logic"
        : "bug";

      const fingerprint = GenericPlatformHarness.computeFingerprint(
        err.name || "StageExecutionError",
        err.message || String(err),
        stageId
      );

      let filedTaskId: string | undefined;
      if (this.coordinator?.autoFileRehearsalIssue) {
        const fileRes = await this.coordinator.autoFileRehearsalIssue({
          title: `[${tag.toUpperCase()}] ${stageName} failed: ${err.message}`,
          repro_steps: [
            `1. Initialize domain spec '${this.spec.id}' (${this.spec.name} v${this.spec.version})`,
            `2. Execute journey stage '${stageId}' (${stageName})`,
            `3. Failure encountered: ${err.message}`,
          ],
          error_text: `${err.message}\n${err.stack || ""}`,
          fingerprint,
          run_id: this.runId,
          tags: [tag],
          journey_step: stageName,
          expected_result: `Stage '${stageName}' concludes successfully according to spec rules`,
          actual_result: err.message,
          screenshot_url: `/evidence/journey-${this.runId}-${stageId}.png`,
        });
        filedTaskId = fileRes?.taskId;
      }

      if (filedTaskId) {
        this.issuesFiled.push(filedTaskId);
      }

      this.stages.push({
        stage_id: stageId,
        name: stageName,
        status: "failed",
        duration_ms: durationMs,
        error: err.message,
        stack: err.stack,
      });
      return false;
    }
  }

  public getRecord(): JourneyRunRecord {
    const hasFailed = this.stages.some(s => s.status === "failed");
    return {
      run_id: this.runId,
      domain_id: this.spec.id,
      spec_hash: this.spec.content_hash,
      scenario: this.scenario,
      status: hasFailed ? "failed" : "passed",
      stages: this.stages,
      issues_filed: this.issuesFiled,
      total_duration_ms: this.stages.reduce((acc, s) => acc + s.duration_ms, 0),
    };
  }
}
```

---

## 4. Example 1: Legacy Production App (Brakets Tournament Engine)

### Scenario
An existing enterprise tournament platform (`apps/brakets-gg/`) running on Cloudflare Workers + Durable Objects + R2 evidence storage.

### 1. Define Domain as Data
Instead of writing custom code for single elimination, double elimination, and Swiss systems, formats are defined as declarative configs in [`cloudflare/src/rehearsal/FormatRegistry.ts`](file:///c:/Users/Ashley/Downloads/s3a/cloudflare/src/rehearsal/FormatRegistry.ts):

```json
{
  "id": "swiss",
  "name": "Swiss System Pairing",
  "version": "1.0.0",
  "author": "Brakets Core Team",
  "content_hash": "sha256_9b8fa21e",
  "signature": "ed25519_sig_format_swiss_v1",
  "teams_per_match": 2,
  "players_per_team": 1,
  "seeding": "standard_bracket",
  "win_condition": { "type": "most_wins" },
  "tie_breakers": ["buchholz", "sonneborn_berger", "game_win_pct"],
  "advancement": { "structure": "swiss_rounds", "rounds": 3 },
  "rematch_rules": "forbidden"
}
```

A Game Pack overrides default options with esports map vetoes or fighting game FT3 rules:

```json
{
  "id": "esports_tactical",
  "name": "Tactical Esports Pack (Valorant & CS2)",
  "version": "1.0.0",
  "default_format_id": "single_elimination",
  "game_metadata": {
    "map_pool": ["Haven", "Ascent", "Bind", "Split", "Lotus", "Sunset", "Abyss"]
  },
  "event_options": {
    "match_rules": {
      "best_of_n": 3,
      "bans_picks": { "enabled": true, "maps_banned_per_team": 2, "map_pick_order": "ban_ban_pick" }
    }
  }
}
```

### 2. The Failing Journey (Missing Score Route)
During rehearsal run `rehearsal_single_elimination_missing_route_8z4q`, scripted players check into Match 1 (CyberViper vs QuantumDrift). The headless client attempts to navigate to `/tournaments/tourn_8z4q/matches/m1/score`.
The router responds with `404 Not Found`.

### 3. Issue Auto-Filing with Deduplication
The generic harness traps the 404, computes fingerprint `fp_routenot_581f3b2a`, and files:
- **Task ID**: `TASK-REHEARSAL-routenot`
- **Tag**: `["needs-page"]`
- **Expected**: Interactive score reporting form rendered with player inputs and submit button.
- **Actual**: `HTTP 404: route /matches/m1/score does not exist in router manifest.`
- **Evidence**: R2 screenshot `/evidence/rehearsal-m1-score-404.png`.

When a 16-team rehearsal hits the same missing score route 5 minutes later, the harness calculates the same fingerprint `fp_routenot_581f3b2a` and increments `repro_count` (now 2), preventing issue spam.

### 4. Agent Claims, Sandboxes & Proposes PR
1. Autonomous agent `agent_builder_02` claims `TASK-REHEARSAL-routenot`.
2. Agent spawns container sandbox: executes `sandbox_build(["npm", "run", "test"])`.
3. Agent writes `ScoreEntryForm.tsx` and exposes the route in `routes/api.ts`.
4. Opens PR `CR-PAGE-DESIGN-mvc427gt`:
   - Title: `fix(routing): Implement interactive match score reporting interface`
   - Diff attached
   - Before trace: `trace_8z4q_missing_route`
   - After trace: `trace_9m1k_score_passed`

### 5. Supervisor Review & Green Replay
1. Ashley and Ward inspect the PR and execute Ed25519 WebCrypto signatures:
   ```bash
   POST /project/s3a-swarm-main/change-requests/CR-PAGE-DESIGN-mvc427gt/approve
   Header: Authorization: Bearer s3a_admin_sec_9f8b2a14e6c7d35
   Body: { "reviewer_pubkey": "7988cfd6...ea", "signature": "a519...3b" }
   ```
2. The harness re-executes the 8-Team Esports Single-Elimination rehearsal (`rehearsal_single_elimination_mvc42avi_bihe`).
3. All 7 stages—signup, fixture generation, map veto, score entry, dispute resolution, advancement, and standings—report `✓ PASSED (100%)`.

---

## 5. Example 2: Greenfield App (New Scaffold & First Journeys)

### Scenario: Greenfield Multi-Carrier Logistics Dispatcher (`DispatchGate`)

You are building a brand-new multi-carrier package dispatch engine from scratch with zero prior code.

### 1. Define Domain as Data (`CarrierPacks.json`)

```json
{
  "id": "express_freight_v1",
  "name": "Overnight Air & Freight Dispatch Policy",
  "version": "1.0.0",
  "content_hash": "sha256_31a7c09d",
  "signature": "ed25519_sig_policy_v1",
  "carriers": [
    { "id": "fedex_priority", "max_weight_lbs": 150, "sla_hours": 24, "rate_per_mile_cents": 140 },
    { "id": "dhl_express", "max_weight_lbs": 70, "sla_hours": 18, "rate_per_mile_cents": 165 }
  ],
  "dispatch_rules": {
    "optimization": "lowest_sla",
    "require_signature": true,
    "max_dispatch_latency_mins": 30
  }
}
```

### 2. Scaffold Journey 0 (The Golden Path)

The greenfield harness is scripted *before* application features exist:

```typescript
// scripts/greenfield-dispatch-journey.mjs
import { GenericPlatformHarness } from "../cloudflare/src/harness/GenericPlatformHarness";

async function runGreenfieldJourney(coordinator) {
  const harness = new GenericPlatformHarness({
    spec: expressFreightPolicy,
    scenario: "golden_path_dispatch",
    coordinator,
    prefix: "dispatch",
  });

  // Stage 1: Order Ingestion
  await harness.executeStage("stage_order_ingest", "Order Ingestion & SKU Validation", async () => {
    return client.post("/orders", { sku: "SKU-990", weight_lbs: 42, destination: "28403" });
  });

  // Stage 2: Carrier Assignment (Optimization Engine)
  await harness.executeStage("stage_carrier_assign", "Carrier Rate & SLA Assignment", async () => {
    return client.post("/orders/SKU-990/assign-carrier");
  });

  // Stage 3: Label Generation & Barcode Webhook
  await harness.executeStage("stage_label_gen", "Dispatch Label & Barcode Generation", async () => {
    return client.post("/orders/SKU-990/generate-label");
  });

  // Stage 4: Proof of Delivery Handshake
  await harness.executeStage("stage_delivery_pod", "Delivery Signature Verification", async () => {
    return client.post("/orders/SKU-990/confirm-delivery", { signature_svg: "<svg>...</svg>" });
  });

  return harness.getRecord();
}
```

### 3. Journey Fails on Blank Scaffold
Running Journey 0 immediately fails at Stage 2 (`Carrier Rate & SLA Assignment`) because `/assign-carrier` has not been implemented.
- **Harness Action**: Traps `RouteNotFoundException (HTTP 404)`.
- **Auto-Filed Task**: `TASK-DISPATCH-assigncarrier` tagged `["needs-page", "needs-logic"]`.
- **Trace Snapshot**: Saved to R2 with payload requirement: `Expected rate optimization across 2 carriers; received 404`.

### 4. Agent Builds the Missing Logic
1. Agent claims `TASK-DISPATCH-assigncarrier`.
2. Agent reads `CarrierPacks.json`, writes `CarrierAssigner.ts` with lowest-SLA sorting logic.
3. Adds route handler `POST /orders/:id/assign-carrier`.
4. Opens PR `CR-DISPATCH-01`:
   - Title: `feat(dispatch): Implement SLA-optimized carrier assignment logic`
   - Touched files: `src/services/CarrierAssigner.ts`, `src/routes/dispatch.ts`
   - Requires human review: `true`

### 5. Supervisor Approval & Green Replay
1. Ashley & Ward review and execute Ed25519 approval.
2. The harness re-runs `runGreenfieldJourney()`.
3. Stage 1: `✓ Passed` (Order SKU validated)
4. Stage 2: `✓ Passed` (Assigned `dhl_express` for 18h SLA)
5. Stage 3: `✓ Passed` (Label PDF generated)
6. Stage 4: `✓ Passed` (Proof of delivery handshake verified)
7. Final Status: `PASSED (100%)`. The first production-grade slice of the greenfield app is verified and complete.

---

## 6. Implementation Checklist & Invariants

| Invariant | Rule | Enforcement Mechanism |
|---|---|---|
| **Domain as Data** | All rules, bracket shapes, and SLAs must be JSON/YAML data. | `FormatRegistry.computeSpecHash()` rejects unhashed/unsigned mutations. |
| **Zero Cashier Risk** | Staging simulations must never hit live payment/ledger APIs. | Isolated test accounts (`is_test_account: true`, balance: 0). |
| **No Ticket Floods** | Identical crashes must not spawn multiple issues. | Stable error fingerprinting: `fp_<type>_<hash>` increments `repro_count`. |
| **No Fake Passes** | Build outputs must show real exit codes and durations. | Streamed `sandbox_build(argv[])` exit codes and R2 logs. |
| **Human Dual-Control** | Autonomous agents cannot merge to main without approval. | Ed25519 WebCrypto supervisor signatures (`SUPERVISOR_PUBKEYS`). |
| **Green Replay Proof** | Issues are only resolved if the identical journey passes. | Automated harness replay before closing issues or PRs. |

---

## 7. Summary Diagram: The Autonomous Delivery Loop

```
┌────────────────────────────────────────────────────────┐
│               1. DOMAIN CONFIGURATION                  │
│   Formats & Rules as Data (Signed, Versioned JSON)     │
└───────────────────────────┬────────────────────────────┘
                            │
                            ▼
┌────────────────────────────────────────────────────────┐
│               2. SIMULATED REHEARSAL                   │
│   Headless Personas, Scripted Journeys, Zero Risk      │
└───────────────────────────┬────────────────────────────┘
                            │
                ┌───────────┴───────────┐
                ▼                       ▼
          [ALL PASSED]             [EXCEPTION]
                │                       │
                ▼                       ▼
    ┌──────────────────────┐  ┌───────────────────────────┐
    │  Production Ready!   │  │  3. DEDUPLICATED AUTO-FIL │
    │  Deploy with Cert    │  │  needs-page / needs-logic │
    └──────────────────────┘  └─────────────┬─────────────┘
                                            │
                                            ▼
                              ┌───────────────────────────┐
                              │  4. AGENT PULL REQUEST    │
                              │  Sandbox Build + Trace    │
                              └─────────────┬─────────────┘
                                            │
                                            ▼
                              ┌───────────────────────────┐
                              │  5. DUAL HUMAN APPROVAL   │
                              │  Ashley + Ward Ed25519    │
                              └─────────────┬─────────────┘
                                            │
                                            ▼
                              ┌───────────────────────────┐
                              │  REPLAY SIMULATION HARNESS│
                              │  Re-verify until Green    │
                              └───────────────────────────┘
```
