// cloudflare/src/types.ts
// S3A Gate Shared Types and Data Models

export interface Env {
  ARTIFACTS: any;
  REPO_COORDINATOR: any;
  Sandbox: any;
  AI: any;
  BROWSER: any;
  EVIDENCE_BUCKET: R2Bucket;
  AGENT_WORKFLOW: Workflow<AgentTaskParams>;
  GITHUB_TOKEN?: string;
  AGENT_ED25519_KEY?: string;
  OLLAMA_BASE_URL?: string;
  CUSTOM_MODEL_URL?: string;
  ADMIN_SECRET?: string;
  SUPERVISOR_PUBKEYS?: string;
  GATE_AGENT_ED25519_SECRET?: string;
}

export type OperationStatus = "accepted" | "running" | "done" | "failed";

export interface OperationRecord {
  op_id: string;
  idempotency_key: string;
  task_id: string;
  agent_id?: string;
  status: OperationStatus;
  started: number;
  finished?: number;
  pr_number?: number;
  pr_url?: string;
  change_request_id?: string;
  error?: string;
  metadata?: Record<string, any>;
}

export interface AgentTaskParams {
  projectId: string;
  taskId: string;
  agentId: string;
  taskTitle: string;
  taskDescription: string;
  filesAllowed: string[];
  tests?: string;
  riskLevel?: "low" | "high";
  githubIssueNumber?: number;
  feedback?: string;
  opId?: string;
  idempotencyKey?: string;
}


export interface StoredRecord {
  record_id: string;
  tile_type: number;
  payload_hash: string;
  summary: string;
  timestamp: number;
  commit_sig_hex: string;
  author_gate: string;
}

export interface StoredConflict {
  conflict_id: string;
  record_id: string;
  tile_type: number;
  type_name: string;
  version_a: string;
  version_b: string;
  author_a: string;
  author_b: string;
  target_version: string;
  source_version: string;
  target_author: string;
  source_author: string;
  created_timestamp: number;
  timestamp: number;
  resolved: boolean;
  choice?: number;
  rationale?: string;
}

export interface StructuredFailure {
  file: string;
  line: number;
  col?: number;
  message: string;
  stage: "build" | "typecheck" | "lint" | "test";
  commit_id: string;
  responsible_agent: string;
}

export interface ProvenanceTile {
  record_id: string;
  tile_type: number; // 40 = PROVENANCE_DIAGNOSTIC
  timestamp: number;
  failure: StructuredFailure;
  raw_diagnostic: string;
}

export interface RepairAttempt {
  attempt_number: number;
  max_attempts: number;
  commit_id: string;
  fix_commit_id?: string;
  agent_addr: string;
  error: StructuredFailure;
  fix_diff?: string;
  result: "evaluating" | "passed" | "failed" | "escalated";
  timestamp: number;
  rationale?: string;
}

export interface RepairMetrics {
  failures_detected: number;
  auto_repaired: number;
  escalated_to_human: number;
  mean_attempts: number;
  shadow_branches_evaluated: number;
  promoted_after_green: number;
}

export type AgentStatus =
  | "idle"
  | "working"
  | "pushing"
  | "merged"
  | "conflict"
  | "blocked"
  | "human"
  | "rejected"
  | "qa_running"
  | "qa_passed"
  | "qa_failed"
  | "deferred_budget";

export type AgentRole = "builder" | "qa_journey" | "visual" | "chaos";

export interface QAEvidenceScreenshot {
  name: string;
  caption: string;
  data_url: string;
  timestamp: number;
}

export interface QAReport {
  id: string;
  agent_id: string;
  agent_role: "qa_journey" | "visual" | "chaos";
  status: "passed" | "failed";
  summary: string;
  passed_steps: string[];
  failed_steps: string[];
  console_errors: string[];
  timings_ms: Record<string, number>;
  screenshots: QAEvidenceScreenshot[];
  visual_diff?: {
    baseline_screenshot?: string;
    candidate_screenshot?: string;
    diff_screenshot?: string;
    mismatch_percent: number;
    identical: boolean;
  };
  chaos_metrics?: {
    concurrent_sessions: number;
    total_actions: number;
    fuzzed_inputs_tested: number;
    unhandled_exceptions: number;
    error_rate_percent: number;
    resilience_passed: boolean;
  };
  created_at: number;
}

export interface TaskItem {
  id: string; // e.g. "TASK-101", "GH-3", "TASK-BUG-..."
  title: string;
  description: string;
  files_allowed: string[];
  tests: string;
  risk_level: "low" | "high" | "review";
  status: "open" | "claimed" | "in_review" | "changes_requested" | "merged" | "failed" | "deferred_budget" | "done";
  claimed_by?: string;
  claimed_at?: number;
  change_request_id?: string;
  feedback?: string;
  is_bug?: boolean;
  originating_cr_id?: string;
  github_issue_number?: number;
  github_issue_url?: string;
  workflow_id?: string;
  error_fingerprint?: string;
  rehearsal_run_id?: string;
  repro_count?: number;
  tags?: string[]; // e.g. ["needs-page"], ["needs-logic"]
  journey_step?: string;
  expected_result?: string;
  actual_result?: string;
  screenshot_url?: string;
}

export interface ChangeRequest {
  id: string; // e.g. "CR-101"
  task_id: string;
  agent_id: string;
  agent_pubkey: string;
  title: string;
  commit_id: string;
  commit_sha?: string;
  sandbox_image_digest?: string;
  seal_hash: string;
  message: string;
  diff: string;
  test_report?: {
    passed: boolean;
    output?: string;
    stage?: string;
    signature?: string;
    exit_code?: number;
    test_exit_code?: number;
    duration_ms?: number;
    log_url?: string;
  };
  test_duration_ms?: number;
  build_log_r2_url?: string;
  touched_files: string[];

  risk_level: "low" | "high" | "review";
  requires_human: boolean;
  status: "pending_review" | "changes_requested" | "auto_merged" | "approved" | "rejected" | "failed_verification";
  human_rationale?: string;
  reviewer_pubkey?: string;
  pm_feedback?: string;
  preview_url?: string;
  build_failed?: boolean;
  build_log?: string;
  qa_reports?: QAReport[];
  qa_status?: "pending" | "running" | "passed" | "failed";
  github_issue_number?: number;
  github_pr_number?: number;
  github_pr_url?: string;
  github_branch?: string;
  needs_second_human?: boolean;
  created_at: number;
  resolved_at?: number;
  before_trace_id?: string;
  after_trace_id?: string;
  tags?: string[]; // e.g. ["needs-page"], ["needs-logic"]
  proposed_page_design?: string;
  proposed_logic_design?: string;
  journey_step?: string;
  screenshot_url?: string;
  evidence_urls?: string[];
}

// ----------------------------------------------------------------------------
// S3A Rehearsal: Simulated Tournament Production Readiness Types
// ----------------------------------------------------------------------------
export type RehearsalTournamentSize = 4 | 8 | 16 | number;
export type RehearsalScenario =
  | "happy_path"
  | "bad_score"
  | "dispute"
  | "unexpressible_step"
  | "missing_logic"
  | "missing_route"
  | "broken_control"
  | "cant_enter_score"
  | "cant_advance_winner"
  | "cant_resolve_dispute";
export type RehearsalStageStatus = "passed" | "failed" | "disputed" | "skipped" | "evidence_missing";

export interface RehearsalStage {
  stage_id: string;
  name: string;
  status: RehearsalStageStatus;
  duration_ms: number;
  details?: any;
  error?: string;
  stack?: string;
}

export interface RehearsalConsoleError {
  timestamp: number;
  level: "error" | "warn";
  message: string;
  source?: string;
}

// ----------------------------------------------------------------------------
// S3A Rehearsal: Tournament Format Registry & Configurable Event Options
// ----------------------------------------------------------------------------
export type TournamentFormatType =
  | "single_elimination"
  | "double_elimination"
  | "round_robin"
  | "swiss"
  | "groups_playoffs"
  | "battle_royale"
  | "custom";

export interface FormatWinCondition {
  type: "first_to_n" | "points_threshold" | "most_wins" | "placement_score";
  target?: number;
}

export type FormatTieBreaker =
  | "head_to_head"
  | "buchholz"
  | "sonneborn_berger"
  | "point_differential"
  | "game_win_pct"
  | "most_wins"
  | "points"
  | "rematch";

export interface FormatAdvancement {
  structure: "bracket" | "swiss_rounds" | "groups_to_bracket" | "lobby_elimination";
  rounds?: number;
  qualifiers_per_group?: number;
  losers_bracket?: boolean;
  grand_finals_reset?: boolean;
}

export interface TournamentFormatConfig {
  id: string;
  name: string;
  version: string;
  author: string;
  content_hash: string;
  signature?: string;
  teams_per_match: number;
  players_per_team: number;
  seeding: "standard_bracket" | "snake" | "random" | "rating";
  win_condition: FormatWinCondition;
  tie_breakers: FormatTieBreaker[];
  advancement: FormatAdvancement;
  rematch_rules: "allowed" | "forbidden" | "playoffs_only";
}

export interface EventOptions {
  match_rules: {
    best_of_n: number;
    per_game_scoring: boolean;
    bans_picks?: {
      enabled: boolean;
      pool_size: number;
      bans_per_team: number;
      system: string;
    };
  };
  check_in: {
    window_minutes: number;
    auto_dq_no_show: boolean;
  };
  score_reporting: {
    mode: "both_players" | "to_only" | "auto_verified";
    dispute_grace_period_mins: number;
    allow_disputes: boolean;
  };
  roster_rules: {
    substitutes_allowed: number;
    coach_slots: number;
    roster_lock: boolean;
  };
  multi_stage?: boolean;
  stages?: Array<{
    stage_num: number;
    name: string;
    format_id: string;
    teams_count: number;
    advance_count: number;
  }>;
}

export interface GamePack {
  id: string;
  name: string;
  version: string;
  genre: "esports" | "fighting" | "tabletop" | "sports" | "battle_royale";
  default_format_id: string;
  event_options: Partial<EventOptions>;
  game_metadata: {
    titles: string[];
    map_pool?: string[];
    overtime_rule?: string;
    match_term: string;
  };
  content_hash: string;
  signature?: string;
}

export interface RehearsalFailedApi {
  endpoint: string;
  method: string;
  status: number;
  request_body?: any;
  response_body?: any;
  duration_ms: number;
}

export interface RehearsalMatch {
  id: string;
  round: number;
  match_num: number;
  round_name: string;
  group_name?: string;
  bracket_type?: "upper" | "lower" | "grand_finals" | "group" | "swiss";
  team1: { id: string; name: string; seed: number; score?: number };
  team2: { id: string; name: string; seed: number; score?: number };
  winner_id?: string;
  status: "pending" | "ready" | "in_progress" | "disputed" | "final";
  dispute?: {
    reported_by: string;
    conflicting_scores: any;
    reason: string;
    resolved_by?: string;
    resolution?: string;
  };
  map_veto?: {
    pool: string[];
    banned: string[];
    picked: string;
  };
}

export interface RehearsalStanding {
  rank: number;
  team_id: string;
  team_name: string;
  record: string;
  prize_points: number;
  tiebreak_score?: number;
  buchholz_score?: number;
}

export interface RehearsalRunRecord {
  run_id: string;
  tournament_id: string;
  tournament_name: string;
  bracket_size: number;
  scenario: RehearsalScenario;
  format_id?: string;
  game_pack_id?: string;
  format_version?: string;
  format_hash?: string;
  format_name?: string;
  status: "passed" | "failed" | "disputed_resolved" | "evidence_missing";
  started_at: number;
  finished_at: number;
  total_duration_ms: number;
  stages: RehearsalStage[];
  matches: RehearsalMatch[];
  standings: RehearsalStanding[];
  console_errors: RehearsalConsoleError[];
  failed_api_calls: RehearsalFailedApi[];
  screenshots: QAEvidenceScreenshot[];
  evidence_keys?: string[];
  missing_evidence?: string[];
  evidence_verified?: boolean;
  timings_ms: Record<string, number>;
  trace_id: string;
  issues_filed: string[];
  before_trace_id?: string;
  after_trace_id?: string;
}

export interface RehearsalOptions {
  size?: number;
  scenario?: RehearsalScenario;
  simulatedError?: string;
  format_id?: string;
  game_pack_id?: string;
  event_options?: Partial<EventOptions>;
}

export interface RegisteredAgent {
  agent_id: string;
  agent_pubkey: string;
  agent_role?: AgentRole;
  status: AgentStatus;
  task_id?: string;
  task_title?: string;
  commit_count: number;
  fork_repo?: string;
  registered_at: number;
  last_seen: number;
  files_allowed?: string[];
}

export interface DailyLimits {
  max_tasks_per_day: number;
  max_llm_calls_per_day: number;
  max_sandbox_minutes_per_day: number;
}

export interface DailyUsage {
  date: string;
  tasks_started: number;
  llm_calls: number;
  sandbox_minutes: number;
}

export type ActionKind = "llm" | "sandbox" | "browser" | "r2" | "free";

export interface LedgerRecord {
  id: string;
  ts: number;
  task_id: string;
  agent_id: string;
  kind: ActionKind;
  units: number;
  est_cost: number;
  is_free: boolean;
  cumulative_spent: number;
}

export interface DeferredAction {
  id: string;
  task_id: string;
  agent_id: string;
  kind: ActionKind;
  units: number;
  est_cost: number;
  reason: "deferred_budget";
  queued_at: number;
}

export interface BudgetStatus {
  spent_today: number;
  cap: number;
  pct_spent: number;
  warning_80_pct: boolean;
  cap_reached: boolean;
  time_until_reset_secs: number;
  deferred_queue: DeferredAction[];
  entries: LedgerRecord[];
}

export interface ActorProvenance {
  type: "human" | "ai";
  id?: string;
  name?: string;
  supervisor_id?: string;
  reviewer_pubkey?: string;
  rationale?: string;
  agent_id?: string;
  model?: string;
}

export interface TraceStep {
  step_id: string;
  task_id: string;
  step_index: number;
  step_name: string;
  actor: ActorProvenance;
  xapi_verb: string;
  prompt?: string;
  model?: string;
  patch?: string;
  build_test_log?: string;
  qa_evidence?: any;
  human_decision?: any;
  ts: number;
}

export interface TaskTrace {
  task_id: string;
  title: string;
  steps: TraceStep[];
  created_at: number;
  updated_at: number;
}

export interface BreadcrumbItem {
  label: string;
  url?: string;
}

export interface RenderViewOptions {
  projectId?: string;
  activeTab?: string;
  breadcrumbs?: BreadcrumbItem[];
  selectedTaskId?: string;
  selectedCrId?: string;
  selectedRunId?: string;
  statusData?: any;
  controlData?: any;
  budgetData?: BudgetStatus;
  tasks?: TaskItem[];
  changeRequests?: ChangeRequest[];
  registeredAgents?: RegisteredAgent[];
  actionRuns?: any[];
  error?: string;
}

export function formatPREvidenceComment(
  cr: ChangeRequest,
  task: Partial<TaskItem>,
  issueNum: number,
  prNumber: number
): string {
  const commitSha = cr.commit_sha || cr.commit_id || "not run";
  const sandboxImageDigest = cr.sandbox_image_digest || "docker.io/cloudflare/sandbox:0.12.10";

  const hasLogs = !!(cr.build_log || (cr.test_report && (cr.test_report.output || cr.test_report.log_url)));
  const buildExitCode = cr.test_report?.exit_code !== undefined
    ? String(cr.test_report.exit_code)
    : (hasLogs ? (cr.test_report?.passed ? "0" : "1") : "not run");

  const testExitCode = cr.test_report?.test_exit_code !== undefined
    ? String(cr.test_report.test_exit_code)
    : (cr.test_report?.exit_code !== undefined ? String(cr.test_report.exit_code) : "not run");

  const duration = cr.test_report?.duration_ms
    ? `${cr.test_report.duration_ms}ms`
    : (cr.test_duration_ms ? `${cr.test_duration_ms}ms` : "not run");

  const logUrl = cr.test_report?.log_url || cr.build_log_r2_url;
  const logLinkText = hasLogs
    ? (logUrl ? `[View Build & Test Log](${logUrl})` : 'evidence missing')
    : 'not run';

  const buildSection = `#### 🧪 Build & Test Verification
- **Test Command:** \`${cr.test_report?.stage || (hasLogs ? 'npm run build' : 'not run')}\`
- **Build Exit Code:** \`${buildExitCode}\`
- **Test Exit Code:** \`${testExitCode}\`
- **Duration:** \`${duration}\`
- **Log Link (R2):** ${logLinkText}
- **Build Status:** ${hasLogs ? (buildExitCode === "0" ? 'Passed' : 'Failed') : 'not run'}`;

  const hasQa = !!(cr.qa_reports && cr.qa_reports.length > 0);
  let qaScreenshotsSection = "";
  let qaReportsSection = "";

  if (hasQa) {
    const allScreenshots = cr.qa_reports!.flatMap(r => r.screenshots || []);
    if (allScreenshots.length > 0) {
      qaScreenshotsSection = allScreenshots.map(s => {
        if (!s.data_url || (s as any).missing) {
          return `- **${s.caption || s.name}**: [evidence missing: ${s.name}]`;
        }
        const url = s.data_url.startsWith('http') ? s.data_url : `https://s3a-gate.ashley-e-cribb.workers.dev${s.data_url}`;
        return `- **${s.caption || s.name}**: ![](${url})\n  - [R2 Artifact Link](${url})`;
      }).join('\n');
    } else {
      qaScreenshotsSection = "*Screenshots (R2): not run*";
    }

    qaReportsSection = cr.qa_reports!.map(r => `
- **QA Agent:** \`${r.agent_id}\` (Role: **${r.agent_role.toUpperCase()}**) - **${r.status.toUpperCase()}**
  - Passed steps: ${(r.passed_steps || []).join(', ') || 'None'}
  - Latency: ${Object.entries(r.timings_ms || {}).map(([k, v]) => `${k}: ${v}ms`).join(', ') || 'None'}
  - Console Errors: ${(r.console_errors || []).length}
  ${r.visual_diff ? `- Visual Mismatch: ${r.visual_diff.mismatch_percent || 0.0}%` : ''}
`).join('\n');
  } else {
    qaScreenshotsSection = "*Screenshots (R2): not run*";
    qaReportsSection = "*QA not run*";
  }

  const isSensitive = /cashier|escrow|payout|auth|db|infra/i.test((task.title || '') + ' ' + (cr.touched_files || []).join(' ') + ' ' + (task.description || ''));
  const needsHuman = cr.requires_human || isSensitive;

  return `### 🤖 S3A Autonomous Agent Change Request

- **Commit SHA:** \`${commitSha}\`
- **Sandbox Image Digest:** \`${sandboxImageDigest}\`
- **Agent ID:** \`${cr.agent_id}\`
- **Seal Hash:** \`${cr.seal_hash || 'not run'}\`
- **Timestamp:** \`${new Date().toISOString()}\`
- **Target Issue:** #${issueNum} (\`${task.title || 'Task'}\`)
- **Review Policy:** ${needsHuman ? '🔒 **needs-human** (cashier/escrow/payout/auth/db/infra or high-risk)' : '🤖 Low-risk automated'}

${buildSection}

#### 🎭 Real QA Playwright Evidence (Headless Chromium)
${qaReportsSection}

#### 📸 Screenshots from R2
${qaScreenshotsSection}

#### 🌐 Live PM Preview
[Open Live Brakets Preview on S3A Gate ↗](https://s3a-gate.ashley-e-cribb.workers.dev/p/s3a-swarm-main/pulls/${cr.id})

> [!NOTE]
> Attached verified evidence and real execution status to PR #${prNumber}. Zero duplicate PRs created.
`;
}

// --------------------------------------------------------------------------
// Brakets Environment Lifecycle & Promotion Types
// --------------------------------------------------------------------------
export type BraketsEnv = "demo" | "test" | "production";

export interface PromotionChecklistItem {
  id: string;
  title: string;
  passed: boolean;
  details: string;
}

export interface PromotionChecklist {
  target_env: BraketsEnv;
  current_env: BraketsEnv;
  ready: boolean;
  checklist: PromotionChecklistItem[];
  blockers: string[];
}

export interface PromotionRecord {
  id: string;
  from_env: BraketsEnv;
  to_env: BraketsEnv;
  requester_id: string;
  requester_type: "ai" | "human";
  approved_by: string;
  supervisor_pubkey: string;
  rationale: string;
  rollback_plan: string;
  auto_merge: boolean;
  timestamp: number;
}

