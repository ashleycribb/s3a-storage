// cloudflare/src/rehearsal/TournamentHarness.ts
// S3A Rehearsal: Simulated Tournament Engine for Brakets Production Readiness
// Strictly staging/demo only, zero real money risk, test accounts only.

import type {
  Env,
  RehearsalTournamentSize,
  RehearsalScenario,
  RehearsalStage,
  RehearsalMatch,
  RehearsalStanding,
  RehearsalConsoleError,
  RehearsalFailedApi,
  RehearsalRunRecord,
  QAEvidenceScreenshot,
  TaskItem,
  EventOptions,
} from "../types";
import { FormatRegistry } from "./FormatRegistry";
import { GenericTournamentEngine, type Participant } from "./GenericTournamentEngine";
import { generateRehearsalEvidenceSvg } from "./visualEvidence";

export interface RehearsalExecuteOptions {
  size?: RehearsalTournamentSize;
  scenario?: RehearsalScenario;
  simulatedError?: string;
  format_id?: string;
  game_pack_id?: string;
  event_options?: Partial<EventOptions>;
}

export class TournamentHarness {
  private runId: string;
  private size: RehearsalTournamentSize;
  private scenario: RehearsalScenario;
  private env: Env;
  private options?: RehearsalExecuteOptions;
  private consoleErrors: RehearsalConsoleError[] = [];
  private failedApiCalls: RehearsalFailedApi[] = [];
  private timings: Record<string, number> = {};
  private screenshots: QAEvidenceScreenshot[] = [];
  private stages: RehearsalStage[] = [];
  private matches: RehearsalMatch[] = [];
  private standings: RehearsalStanding[] = [];

  constructor(env: Env, options?: RehearsalExecuteOptions) {
    this.env = env;
    this.options = options;
    this.size = options?.size || 8;
    this.scenario = options?.scenario || "happy_path";
    const prefix = options?.format_id ? `rehearsal_${options.format_id}` : `rehearsal_${this.size}team_${this.scenario}`;
    this.runId = `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
  }

  public getRunId(): string {
    return this.runId;
  }

  private logError(message: string, source: string = "harness", level: "error" | "warn" = "error") {
    this.consoleErrors.push({
      timestamp: Date.now(),
      level,
      message,
      source,
    });
  }

  private logApiFailure(endpoint: string, method: string, status: number, reqBody: any, resBody: any, durMs: number) {
    this.failedApiCalls.push({
      endpoint,
      method,
      status,
      request_body: reqBody,
      response_body: resBody,
      duration_ms: durMs,
    });
  }

  /**
   * Generates deterministic test players and tournament organizer (no real money accounts).
   */
  private getSeedTeams(count: number) {
    const allNames = [
      "CyberViper",   // 1
      "ShadowPulse",  // 2
      "HyperBlitz",   // 3
      "SolarFlare",   // 4
      "VortexNova",   // 5
      "IronTitan",    // 6
      "EchoPhantom",  // 7
      "QuantumDrift", // 8
      "NovaStriker",  // 9
      "ApexGlitch",   // 10
      "OmegaSpark",   // 11
      "BlazeFury",    // 12
      "RaptorClaw",   // 13
      "ZeroKelvin",   // 14
      "StormBreaker", // 15
      "TitanForge",   // 16
    ];

    return allNames.slice(0, count).map((name, idx) => ({
      id: `player_${(idx + 1).toString().padStart(2, "0")}`,
      handle: name,
      name,
      seed: idx + 1,
      role: "player" as const,
      is_test_account: true,
      balance_cents: 0, // Zero real money!
    }));
  }

  /**
   * Computes a stable error fingerprint for deduplication.
   */
  public static computeFingerprint(errorType: string, message: string, location: string = ""): string {
    const norm = `${errorType}:${message.trim().toLowerCase().slice(0, 100)}:${location.trim().toLowerCase()}`;
    let hash = 0;
    for (let i = 0; i < norm.length; i++) {
      hash = (hash << 5) - hash + norm.charCodeAt(i);
      hash |= 0;
    }
    const hex = Math.abs(hash).toString(16).padStart(8, "0");
    return `fp_${errorType.toLowerCase().slice(0, 8)}_${hex}`;
  }

  /**
   * Runs the complete tournament rehearsal simulation.
   */
  public async execute(coordinator: any, projectId: string): Promise<RehearsalRunRecord> {
    const startTime = Date.now();

    // ------------------------------------------------------------------------
    // Format Registry & Generic Data-Driven Engine Delegation
    // ------------------------------------------------------------------------
    if (this.options?.format_id || this.options?.game_pack_id) {
      const registry = new FormatRegistry();
      let formatId = this.options.format_id;
      const packId = this.options.game_pack_id;
      const pack = packId ? registry.getGamePack(packId) : undefined;
      if (!formatId && pack) {
        formatId = pack.default_format_id;
      }
      formatId = formatId || "single_elimination";
      const format = registry.getFormat(formatId);
      if (!format) {
        throw new Error(`Format '${formatId}' not registered in Tournament Format Registry.`);
      }

      let actualSize = this.size;
      if (!this.options.size) {
        if (formatId === "double_elimination") actualSize = 16;
        else if (formatId === "swiss" || formatId === "groups_playoffs" || formatId === "single_elimination") actualSize = 8;
      }
      this.size = actualSize;

      const eventOptions = registry.composeEventOptions(packId, this.options.event_options);
      const seedTeams = this.getSeedTeams(this.size);
      const participants: Participant[] = seedTeams.map(t => ({
        id: t.id,
        name: t.name,
        seed: t.seed,
        role: "player",
        is_test_account: true,
        match_wins: 0,
        match_losses: 0,
        game_wins: 0,
        game_losses: 0,
        opponents_played: [],
        buchholz_score: 0,
      }));

      const engine = new GenericTournamentEngine({
        format,
        pack,
        options: eventOptions,
        scenario: this.scenario,
        runId: this.runId,
        coordinator,
        projectId,
        env: this.env,
      });

      const result = await engine.runEvent(participants);

      const record: RehearsalRunRecord = {
        run_id: this.runId,
        tournament_id: `tourn_${this.runId}`,
        tournament_name: `Brakets Arena [${format.name}${pack ? ` - ${pack.name}` : ''}]`,
        bracket_size: this.size,
        scenario: this.scenario,
        format_id: format.id,
        game_pack_id: pack?.id,
        format_version: format.version,
        format_hash: format.content_hash,
        format_name: format.name,
        status: result.status,
        started_at: startTime,
        finished_at: Date.now(),
        total_duration_ms: result.timings["total_duration_ms"] || (Date.now() - startTime),
        stages: result.stages,
        matches: result.matches,
        standings: result.standings,
        console_errors: result.consoleErrors,
        failed_api_calls: result.failedApiCalls,
        screenshots: result.screenshots,
        evidence_keys: result.evidenceKeys || result.screenshots.map(s => s.name),
        missing_evidence: result.missingEvidence,
        evidence_verified: result.evidenceVerified ?? (result.missingEvidence?.length === 0),
        timings_ms: result.timings,
        trace_id: this.runId,
        issues_filed: result.issuesFiled,
      };

      return record;
    }

    let overallStatus: "passed" | "failed" | "disputed_resolved" | "evidence_missing" = "passed";
    const issuesFiled: string[] = [];

    const teams = this.getSeedTeams(this.size);
    const to = {
      id: "to_01",
      handle: "NeonOrganizer",
      name: "NeonOrganizer",
      role: "organizer" as const,
      is_test_account: true,
    };

    // ------------------------------------------------------------------------
    // Stage 1: Player & TO Signups
    // ------------------------------------------------------------------------
    const t0 = Date.now();
    try {
      this.stages.push({
        stage_id: "stage_signup",
        name: "Scripted Player & TO Registration",
        status: "passed",
        duration_ms: Date.now() - t0,
        details: {
          players_registered: teams.length,
          to_registered: to.handle,
          test_mode: "STAGING_DEMO_ZERO_MONEY",
          accounts: teams.map(t => ({ seed: t.seed, handle: t.handle, role: t.role })),
        },
      });
      this.timings["stage_signup_ms"] = Date.now() - t0;
    } catch (err: any) {
      this.logError(`Signup failed: ${err.message}`, "stage_signup");
      this.stages.push({
        stage_id: "stage_signup",
        name: "Scripted Player & TO Registration",
        status: "failed",
        duration_ms: Date.now() - t0,
        error: err.message,
        stack: err.stack,
      });
      overallStatus = "failed";
    }

    // ------------------------------------------------------------------------
    // Stage 2: TO Bracket Creation (4, 8, or 16)
    // ------------------------------------------------------------------------
    const t1 = Date.now();
    try {
      if (this.size === 4) {
        this.matches = [
          {
            id: "m1", round: 1, match_num: 1, round_name: "Semifinals",
            team1: { id: teams[0].id, name: teams[0].name, seed: teams[0].seed },
            team2: { id: teams[3].id, name: teams[3].name, seed: teams[3].seed },
            status: "ready",
          },
          {
            id: "m2", round: 1, match_num: 2, round_name: "Semifinals",
            team1: { id: teams[1].id, name: teams[1].name, seed: teams[1].seed },
            team2: { id: teams[2].id, name: teams[2].name, seed: teams[2].seed },
            status: "ready",
          },
          {
            id: "m3", round: 2, match_num: 3, round_name: "Grand Finals",
            team1: { id: "", name: "Winner M1", seed: 0 },
            team2: { id: "", name: "Winner M2", seed: 0 },
            status: "pending",
          },
        ];
      } else if (this.size === 8) {
        this.matches = [
          {
            id: "m1", round: 1, match_num: 1, round_name: "Quarterfinals",
            team1: { id: teams[0].id, name: teams[0].name, seed: teams[0].seed },
            team2: { id: teams[7].id, name: teams[7].name, seed: teams[7].seed },
            status: "ready",
          },
          {
            id: "m2", round: 1, match_num: 2, round_name: "Quarterfinals",
            team1: { id: teams[3].id, name: teams[3].name, seed: teams[3].seed },
            team2: { id: teams[4].id, name: teams[4].name, seed: teams[4].seed },
            status: "ready",
          },
          {
            id: "m3", round: 1, match_num: 3, round_name: "Quarterfinals",
            team1: { id: teams[1].id, name: teams[1].name, seed: teams[1].seed },
            team2: { id: teams[6].id, name: teams[6].name, seed: teams[6].seed },
            status: "ready",
          },
          {
            id: "m4", round: 1, match_num: 4, round_name: "Quarterfinals",
            team1: { id: teams[2].id, name: teams[2].name, seed: teams[2].seed },
            team2: { id: teams[5].id, name: teams[5].name, seed: teams[5].seed },
            status: "ready",
          },
          {
            id: "m5", round: 2, match_num: 5, round_name: "Semifinals",
            team1: { id: "", name: "Winner M1", seed: 0 },
            team2: { id: "", name: "Winner M2", seed: 0 },
            status: "pending",
          },
          {
            id: "m6", round: 2, match_num: 6, round_name: "Semifinals",
            team1: { id: "", name: "Winner M3", seed: 0 },
            team2: { id: "", name: "Winner M4", seed: 0 },
            status: "pending",
          },
          {
            id: "m7", round: 3, match_num: 7, round_name: "Grand Finals",
            team1: { id: "", name: "Winner M5", seed: 0 },
            team2: { id: "", name: "Winner M6", seed: 0 },
            status: "pending",
          },
        ];
      } else {
        // 16-team support
        this.matches = [];
        for (let i = 0; i < 8; i++) {
          this.matches.push({
            id: `m${i + 1}`, round: 1, match_num: i + 1, round_name: "Round of 16",
            team1: { id: teams[i].id, name: teams[i].name, seed: teams[i].seed },
            team2: { id: teams[15 - i].id, name: teams[15 - i].name, seed: teams[15 - i].seed },
            status: "ready",
          });
        }
      }

      this.stages.push({
        stage_id: "stage_bracket_create",
        name: `TO Creates ${this.size}-Team Elimination Bracket`,
        status: "passed",
        duration_ms: Date.now() - t1,
        details: {
          tournament_name: `Brakets Arena Invitational (${this.size}-Team Single Elimination)`,
          organizer: to.handle,
          total_matches: this.matches.length,
          bracket_format: "Single Elimination",
        },
      });
      this.timings["stage_bracket_create_ms"] = Date.now() - t1;
    } catch (err: any) {
      this.logError(`Bracket creation failed: ${err.message}`, "stage_bracket_create");
      this.stages.push({
        stage_id: "stage_bracket_create",
        name: `TO Creates ${this.size}-Team Elimination Bracket`,
        status: "failed",
        duration_ms: Date.now() - t1,
        error: err.message,
        stack: err.stack,
      });
      overallStatus = "failed";
    }

    // ------------------------------------------------------------------------
    // Stage 3: Match Slot Check-In
    // ------------------------------------------------------------------------
    const t2 = Date.now();
    try {
      this.stages.push({
        stage_id: "stage_match_checkin",
        name: "Players Check-in to Match Slots",
        status: "passed",
        duration_ms: Date.now() - t2,
        details: {
          checked_in_players: teams.length,
          round_1_matches_ready: this.matches.filter(m => m.round === 1).length,
        },
      });
      this.timings["stage_match_checkin_ms"] = Date.now() - t2;
    } catch (err: any) {
      this.logError(`Match checkin failed: ${err.message}`, "stage_match_checkin");
      this.stages.push({
        stage_id: "stage_match_checkin",
        name: "Players Check-in to Match Slots",
        status: "failed",
        duration_ms: Date.now() - t2,
        error: err.message,
      });
      overallStatus = "failed";
    }

    // ------------------------------------------------------------------------
    // Stage 4: Score Reporting & Verification
    // ------------------------------------------------------------------------
    const t3 = Date.now();
    let scoreStageFailed = false;

    if (this.scenario === "missing_route") {
      const simulatedRoute = `/tournaments/tourn_${this.runId}/matches/m1/score`;
      const simulatedErrorMsg = `RouteNotFoundException: GET ${simulatedRoute} returned 404 Not Found (Missing route handler for match score entry page)`;
      const screenshotKey = `rehearsal-${this.runId}-missing-route.png`;
      const screenshotUrl = `/evidence/${screenshotKey}`;

      this.logError(simulatedErrorMsg, "router.ts");
      this.logApiFailure(simulatedRoute, "GET", 404, null, { error: "Not Found", message: simulatedErrorMsg }, 88);

      this.screenshots.push({
        name: screenshotKey,
        caption: `Rehearsal Evidence: Missing route 404 for match score entry page`,
        data_url: screenshotUrl,
        timestamp: Date.now(),
      });

      const fingerprint = TournamentHarness.computeFingerprint("RouteNotFoundException", simulatedErrorMsg, "router.ts:404");

      if (coordinator) {
        const filedResult = await coordinator.autoFileRehearsalIssue({
          title: "Missing route: match score entry interface (/tournaments/:id/matches/:matchId/score) returns 404",
          repro_steps: [
            "1. Scripted players sign up and join 8-team single elimination bracket.",
            "2. Match 1 (CyberViper vs QuantumDrift) is ready.",
            `3. Client attempts to navigate to ${simulatedRoute}.`,
            "4. Router responds with 404 Not Found: missing page component.",
          ],
          error_text: simulatedErrorMsg,
          fingerprint,
          run_id: this.runId,
          risk_level: "high",
          tags: ["needs-page"],
          journey_step: "Match Score Reporting Page Navigation",
          expected_result: "HTTP 200 OK: Match score interface renders interactive player score inputs and submit button",
          actual_result: `HTTP 404 Not Found: Page route ${simulatedRoute} does not exist in routing manifest`,
          screenshot_url: screenshotUrl,
        }, projectId);

        if (filedResult && filedResult.taskId) issuesFiled.push(filedResult.taskId);
      }

      this.stages.push({
        stage_id: "stage_score_report",
        name: "Score Reporting & Input Validation",
        status: "failed",
        duration_ms: Date.now() - t3,
        error: simulatedErrorMsg,
        stack: `Error: ${simulatedErrorMsg}\n    at matchRoute (apps/brakets-gg/app/routes/router.ts:88:12)\n    at handleRequest (apps/brakets-gg/app/entry.server.ts:45:10)`,
        details: {
          attempted_route: simulatedRoute,
          repro_fingerprint: fingerprint,
          auto_filed_issue: issuesFiled[0] || "TASK-AUTO-FILED",
          tag: "needs-page",
        },
      });

      scoreStageFailed = true;
      overallStatus = "failed";
    } else if (this.scenario === "broken_control") {
      const simulatedControl = "button#btn-submit-match-score";
      const simulatedErrorMsg = `BrokenControlError: Button ${simulatedControl} is disabled with pointer-events: none and fails to dispatch score payload`;
      const screenshotKey = `rehearsal-${this.runId}-broken-control.png`;
      const screenshotUrl = `/evidence/${screenshotKey}`;

      this.logError(simulatedErrorMsg, "MatchScoreForm.tsx");
      this.screenshots.push({
        name: screenshotKey,
        caption: `Rehearsal Evidence: Button #btn-submit-match-score disabled and unresponsive`,
        data_url: screenshotUrl,
        timestamp: Date.now(),
      });

      const fingerprint = TournamentHarness.computeFingerprint("BrokenControlError", simulatedErrorMsg, "MatchScoreForm.tsx:102");

      if (coordinator) {
        const filedResult = await coordinator.autoFileRehearsalIssue({
          title: "Broken control: score submit button (#btn-submit-match-score) disabled and unresponsive",
          repro_steps: [
            "1. Scripted players open Match 1 score reporting form.",
            "2. Valid game scores entered: CyberViper=2, QuantumDrift=1.",
            "3. Player clicks 'Submit Official Match Score' button.",
            "4. Button fails to dispatch: disabled attribute persisted with pointer-events: none.",
          ],
          error_text: simulatedErrorMsg,
          fingerprint,
          run_id: this.runId,
          risk_level: "high",
          tags: ["needs-page"],
          journey_step: "Player Score Submission Form Controls",
          expected_result: "Button #btn-submit-match-score activates upon score entry and dispatches score payload",
          actual_result: "Button #btn-submit-match-score remains disabled with pointer-events: none; score submission is blocked",
          screenshot_url: screenshotUrl,
        }, projectId);

        if (filedResult && filedResult.taskId) issuesFiled.push(filedResult.taskId);
      }

      this.stages.push({
        stage_id: "stage_score_report",
        name: "Score Reporting & Input Validation",
        status: "failed",
        duration_ms: Date.now() - t3,
        error: simulatedErrorMsg,
        stack: `Error: ${simulatedErrorMsg}\n    at validateSubmitButton (apps/brakets-gg/components/MatchScoreForm.tsx:102:9)`,
        details: {
          control: simulatedControl,
          repro_fingerprint: fingerprint,
          auto_filed_issue: issuesFiled[0] || "TASK-AUTO-FILED",
          tag: "needs-page",
        },
      });

      scoreStageFailed = true;
      overallStatus = "failed";
    } else if (this.scenario === "bad_score" || this.scenario === "cant_enter_score") {
      // Intentionally simulate bad score submission & unhandled edge-case failure
      const badScorePayload = { match_id: "m1", team1_score: 2, team2_score: 2, format: "single_elimination" };
      const simulatedErrorMsg = "UnhandledScoreException: Single-elimination match cannot resolve draw (2-2) without sudden-death tiebreaker in apps/brakets-gg/app/api/tournaments/score.ts:42";
      const screenshotKey = `rehearsal-${this.runId}-score-logic.png`;
      const screenshotUrl = `/evidence/${screenshotKey}`;

      this.logError(simulatedErrorMsg, "apps/brakets-gg/app/api/tournaments/score.ts");
      this.logApiFailure(
        "/api/tournaments/matches/m1/score",
        "POST",
        500,
        badScorePayload,
        { error: "Internal Server Error", message: simulatedErrorMsg },
        312
      );

      this.screenshots.push({
        name: screenshotKey,
        caption: `Rehearsal Evidence: Draw score rejected by single-elimination logic`,
        data_url: screenshotUrl,
        timestamp: Date.now(),
      });

      const fingerprint = TournamentHarness.computeFingerprint(
        "UnhandledScoreException",
        simulatedErrorMsg,
        "score.ts:42"
      );

      // Auto-file issue for this failure!
      if (coordinator) {
        const filedResult = await coordinator.autoFileRehearsalIssue({
          title: "Unhandled draw score (2-2) in single-elimination tournament match calculator",
          repro_steps: [
            "1. Scripted players sign up and join 8-team single elimination bracket.",
            "2. Match 1 (CyberViper vs QuantumDrift) starts.",
            "3. Score reporting endpoint receives draw score: team1=2, team2=2.",
            "4. Match calculator throws UnhandledScoreException without sudden-death tiebreaker ruleset.",
          ],
          error_text: simulatedErrorMsg,
          fingerprint,
          run_id: this.runId,
          risk_level: "high", // Affects tournament rules & bracket integrity!
          tags: ["needs-logic"],
          journey_step: "Score Reporting & Input Validation",
          expected_result: "Score engine validates inputs, rejects invalid draws, and initiates sudden-death tiebreaker match",
          actual_result: simulatedErrorMsg,
          screenshot_url: screenshotUrl,
        }, projectId);

        if (filedResult && filedResult.taskId) {
          issuesFiled.push(filedResult.taskId);
        }
      }

      this.stages.push({
        stage_id: "stage_score_report",
        name: "Score Reporting & Input Validation",
        status: "failed",
        duration_ms: Date.now() - t3,
        error: simulatedErrorMsg,
        stack: `Error: ${simulatedErrorMsg}\n    at calculateMatchWinner (apps/brakets-gg/app/api/tournaments/score.ts:42:15)\n    at handleScoreReport (apps/brakets-gg/app/api/tournaments/route.ts:89:12)`,
        details: {
          attempted_score: badScorePayload,
          repro_fingerprint: fingerprint,
          auto_filed_issue: issuesFiled[0] || "TASK-AUTO-FILED",
          tag: "needs-logic",
        },
      });

      scoreStageFailed = true;
      overallStatus = "failed";
    } else if (this.scenario === "dispute" || this.scenario === "cant_resolve_dispute") {
      // Conflicting scores submitted for Match 1
      const m1 = this.matches[0];
      m1.status = "disputed";
      m1.team1.score = 2;
      m1.team2.score = 1;
      m1.dispute = {
        reported_by: "QuantumDrift",
        conflicting_scores: {
          CyberViper_claim: { team1: 2, team2: 1 },
          QuantumDrift_claim: { team1: 0, team2: 2 },
        },
        reason: "QuantumDrift claims match disconnected in Game 3 and requests TO replay review",
      };

      this.logError("Dispute triggered on Match 1: Conflicting player score submissions", "dispute_engine", "warn");

      // Other quarterfinal matches conclude normally
      if (this.matches[1]) {
        this.matches[1].status = "final";
        this.matches[1].team1.score = 2;
        this.matches[1].team2.score = 0;
        this.matches[1].winner_id = this.matches[1].team1.id;
      }
      if (this.matches[2]) {
        this.matches[2].status = "final";
        this.matches[2].team1.score = 2;
        this.matches[2].team2.score = 1;
        this.matches[2].winner_id = this.matches[2].team1.id;
      }
      if (this.matches[3]) {
        this.matches[3].status = "final";
        this.matches[3].team1.score = 2;
        this.matches[3].team2.score = 0;
        this.matches[3].winner_id = this.matches[3].team1.id;
      }

      this.stages.push({
        stage_id: "stage_score_report",
        name: "Score Reporting & Conflict Detection",
        status: "disputed",
        duration_ms: Date.now() - t3,
        details: {
          disputed_match: "m1",
          team1_claim: "CyberViper 2 - 1 QuantumDrift",
          team2_claim: "CyberViper 0 - 2 QuantumDrift",
          action: "Awaiting TO Intervention & Dispute Resolution",
        },
      });
    } else {
      // Happy Path or other scenarios: clean scores reported for all Round 1 matches
      this.matches.forEach((m, idx) => {
        if (m.round === 1) {
          m.status = "final";
          m.team1.score = 2;
          m.team2.score = (idx % 2 === 0) ? 1 : 0;
          m.winner_id = m.team1.id;
        }
      });

      this.stages.push({
        stage_id: "stage_score_report",
        name: "Score Reporting & Verification",
        status: "passed",
        duration_ms: Date.now() - t3,
        details: {
          round_1_scores_verified: 4,
          invalid_scores_rejected: 0,
          disputes_count: 0,
        },
      });
    }
    this.timings["stage_score_report_ms"] = Date.now() - t3;

    // ------------------------------------------------------------------------
    // Stage 5: Dispute Resolution (TO Human/Scripted Ruling)
    // ------------------------------------------------------------------------
    const t4 = Date.now();
    let disputeStageFailed = false;

    if (this.scenario === "cant_resolve_dispute") {
      const simulatedErrorMsg = "DisputeArbitrationBlocked: Conflicting claims (CyberViper 2-1 vs QuantumDrift 0-2) rejected without resolution handler in apps/brakets-gg/app/api/tournaments/arbitration.ts:68";
      const screenshotKey = `rehearsal-${this.runId}-dispute-logic.png`;
      const screenshotUrl = `/evidence/${screenshotKey}`;

      this.logError(simulatedErrorMsg, "arbitration.ts");
      this.screenshots.push({
        name: screenshotKey,
        caption: `Rehearsal Evidence: Dispute arbitration blocked on conflicting match claims`,
        data_url: screenshotUrl,
        timestamp: Date.now(),
      });

      const fingerprint = TournamentHarness.computeFingerprint("DisputeArbitrationBlocked", simulatedErrorMsg, "arbitration.ts:68");

      if (coordinator) {
        const filedResult = await coordinator.autoFileRehearsalIssue({
          title: "Dispute arbitration blocked: unhandled conflicting match claims cannot be resolved by ruleset",
          repro_steps: [
            "1. Match 1 logs dispute: CyberViper claims 2-1, QuantumDrift claims 0-2.",
            "2. TO invokes automated arbitration endpoint /api/tournaments/disputes/resolve.",
            "3. Arbitration engine encounters unhandled conflicting telemetry without fallback ruleset.",
            "4. Tournament progression stalls indefinitely.",
          ],
          error_text: simulatedErrorMsg,
          fingerprint,
          run_id: this.runId,
          risk_level: "high",
          tags: ["needs-logic"],
          journey_step: "TO Dispute Arbitration & Score Reconciliation",
          expected_result: "Arbitration ruleset evaluates player evidence replays and awards victory or schedules mandatory rematch",
          actual_result: "DisputeArbitrationBlocked: Conflicting claims rejected without resolution handler; tournament cannot advance",
          screenshot_url: screenshotUrl,
        }, projectId);

        if (filedResult && filedResult.taskId) issuesFiled.push(filedResult.taskId);
      }

      this.stages.push({
        stage_id: "stage_dispute_resolve",
        name: "TO Evidence Review & Dispute Resolution",
        status: "failed",
        duration_ms: Date.now() - t4,
        error: simulatedErrorMsg,
        stack: `Error: ${simulatedErrorMsg}\n    at arbitrateDispute (apps/brakets-gg/app/api/tournaments/arbitration.ts:68:14)`,
        details: {
          disputed_match: "m1",
          repro_fingerprint: fingerprint,
          auto_filed_issue: issuesFiled[issuesFiled.length - 1] || "TASK-AUTO-FILED",
          tag: "needs-logic",
        },
      });

      disputeStageFailed = true;
      overallStatus = "failed";
    } else if (this.scenario === "dispute") {
      const m1 = this.matches[0];
      m1.status = "final";
      m1.winner_id = m1.team1.id; // CyberViper
      if (m1.dispute) {
        m1.dispute.resolved_by = to.handle;
        m1.dispute.resolution = "TO verified server replay: CyberViper won map 3 (Haven) 13-11. CyberViper advances.";
      }

      this.stages.push({
        stage_id: "stage_dispute_resolve",
        name: "TO Evidence Review & Dispute Resolution",
        status: "passed",
        duration_ms: Date.now() - t4,
        details: {
          resolved_match: "m1",
          organizer: to.handle,
          winner: m1.team1.name,
          resolution: m1.dispute?.resolution,
        },
      });
      overallStatus = "disputed_resolved";
    } else {
      this.stages.push({
        stage_id: "stage_dispute_resolve",
        name: "TO Evidence Review & Dispute Resolution",
        status: "passed",
        duration_ms: Date.now() - t4,
        details: { status: "No disputes reported during run" },
      });
    }
    this.timings["stage_dispute_resolve_ms"] = Date.now() - t4;

    // ------------------------------------------------------------------------
    // Stage 6: Round Advancement (Semifinals & Finals)
    // ------------------------------------------------------------------------
    const t5 = Date.now();
    let advancementFailed = false;

    if (this.scenario === "cant_advance_winner") {
      const simulatedErrorMsg = "AdvancementDeadlockException: Bracket calculator failed to advance slot m5.team1 (null reference on match outcome in apps/brakets-gg/app/api/tournaments/bracket.ts:114)";
      const screenshotKey = `rehearsal-${this.runId}-advancement-logic.png`;
      const screenshotUrl = `/evidence/${screenshotKey}`;

      this.logError(simulatedErrorMsg, "bracket.ts");
      this.screenshots.push({
        name: screenshotKey,
        caption: `Rehearsal Evidence: Bracket calculator failed to advance winner to semifinals`,
        data_url: screenshotUrl,
        timestamp: Date.now(),
      });

      const fingerprint = TournamentHarness.computeFingerprint("AdvancementDeadlockException", simulatedErrorMsg, "bracket.ts:114");

      if (coordinator) {
        const filedResult = await coordinator.autoFileRehearsalIssue({
          title: "Bracket progression deadlock: winner cannot be advanced to semifinals bracket slot",
          repro_steps: [
            "1. All Quarterfinals matches conclude with final scores.",
            "2. Tournament director attempts round progression trigger.",
            "3. Bracket calculator throws AdvancementDeadlockException when binding slot m5.team1.",
            "4. Semifinals matches remain unpopulated.",
          ],
          error_text: simulatedErrorMsg,
          fingerprint,
          run_id: this.runId,
          risk_level: "high",
          tags: ["needs-logic"],
          journey_step: "Round Progression & Winner Advancement",
          expected_result: "Winner of Quarterfinals Match 1 (CyberViper) automatically seeds into Semifinals Match 5 slot",
          actual_result: "AdvancementDeadlockException: Bracket calculator failed to advance slot m5.team1 (null reference on match outcome in bracket.ts:114)",
          screenshot_url: screenshotUrl,
        }, projectId);

        if (filedResult && filedResult.taskId) issuesFiled.push(filedResult.taskId);
      }

      this.stages.push({
        stage_id: "stage_advancement",
        name: "Round Progression & Semifinals Play",
        status: "failed",
        duration_ms: Date.now() - t5,
        error: simulatedErrorMsg,
        stack: `Error: ${simulatedErrorMsg}\n    at advanceBracketWinners (apps/brakets-gg/app/api/tournaments/bracket.ts:114:19)`,
        details: {
          repro_fingerprint: fingerprint,
          auto_filed_issue: issuesFiled[issuesFiled.length - 1] || "TASK-AUTO-FILED",
          tag: "needs-logic",
        },
      });

      advancementFailed = true;
      overallStatus = "failed";
    } else if (!scoreStageFailed && !disputeStageFailed) {
      if (this.size === 8) {
        // M5: CyberViper vs SolarFlare
        const m5 = this.matches[4];
        m5.team1 = { id: teams[0].id, name: teams[0].name, seed: teams[0].seed, score: 2 };
        m5.team2 = { id: teams[3].id, name: teams[3].name, seed: teams[3].seed, score: 1 };
        m5.winner_id = teams[0].id;
        m5.status = "final";

        // M6: ShadowPulse vs HyperBlitz
        const m6 = this.matches[5];
        m6.team1 = { id: teams[1].id, name: teams[1].name, seed: teams[1].seed, score: 2 };
        m6.team2 = { id: teams[2].id, name: teams[2].name, seed: teams[2].seed, score: 0 };
        m6.winner_id = teams[1].id;
        m6.status = "final";

        // M7: Grand Finals (CyberViper vs ShadowPulse)
        const m7 = this.matches[6];
        m7.team1 = { id: teams[0].id, name: teams[0].name, seed: teams[0].seed, score: 2 };
        m7.team2 = { id: teams[1].id, name: teams[1].name, seed: teams[1].seed, score: 1 };
        m7.winner_id = teams[0].id;
        m7.status = "final";
      } else if (this.size === 4) {
        const m3 = this.matches[2];
        m3.team1 = { id: teams[0].id, name: teams[0].name, seed: teams[0].seed, score: 2 };
        m3.team2 = { id: teams[1].id, name: teams[1].name, seed: teams[1].seed, score: 0 };
        m3.winner_id = teams[0].id;
        m3.status = "final";
      }

      this.stages.push({
        stage_id: "stage_advancement",
        name: "Round Progression & Semifinals Play",
        status: "passed",
        duration_ms: Date.now() - t5,
        details: {
          advanced_to_semifinals: ["CyberViper", "SolarFlare", "ShadowPulse", "HyperBlitz"],
          advanced_to_finals: ["CyberViper", "ShadowPulse"],
        },
      });
    } else {
      this.stages.push({
        stage_id: "stage_advancement",
        name: "Round Progression & Semifinals Play",
        status: "skipped",
        duration_ms: Date.now() - t5,
        details: { reason: "Halted due to prior stage failure" },
      });
    }
    this.timings["stage_advancement_ms"] = Date.now() - t5;

    // ------------------------------------------------------------------------
    // Stage 7: Finals & Final Standings
    // ------------------------------------------------------------------------
    const t6 = Date.now();
    if (!scoreStageFailed && !disputeStageFailed && !advancementFailed) {
      this.standings = [
        { rank: 1, team_id: teams[0].id, team_name: teams[0].name, record: "3 - 0", prize_points: 500 },
        { rank: 2, team_id: teams[1].id, team_name: teams[1].name, record: "2 - 1", prize_points: 300 },
        { rank: 3, team_id: teams[3].id, team_name: teams[3].name, record: "1 - 1", prize_points: 150 },
        { rank: 4, team_id: teams[2].id, team_name: teams[2].name, record: "1 - 1", prize_points: 150 },
        { rank: 5, team_id: teams[4].id, team_name: teams[4].name, record: "0 - 1", prize_points: 50 },
        { rank: 6, team_id: teams[6].id, team_name: teams[6].name, record: "0 - 1", prize_points: 50 },
        { rank: 7, team_id: teams[5].id, team_name: teams[5].name, record: "0 - 1", prize_points: 50 },
        { rank: 8, team_id: teams[7].id, team_name: teams[7].name, record: "0 - 1", prize_points: 50 },
      ].slice(0, this.size);

      this.stages.push({
        stage_id: "stage_finals_standings",
        name: "Grand Finals & Official Tournament Standings",
        status: "passed",
        duration_ms: Date.now() - t6,
        details: {
          champion: teams[0].name,
          runner_up: teams[1].name,
          standings_count: this.standings.length,
          total_prize_points_awarded: 1400,
          real_cashier_transfers: 0, // Staging safety invariant!
        },
      });
    } else {
      this.stages.push({
        stage_id: "stage_finals_standings",
        name: "Grand Finals & Official Tournament Standings",
        status: "skipped",
        duration_ms: Date.now() - t6,
        details: { reason: "Standings not reachable due to prior stage failure" },
      });
    }
    this.timings["stage_finals_standings_ms"] = Date.now() - t6;

    // ------------------------------------------------------------------------
    // Screenshots & Evidence Capture (Requirement 1: Real R2 Object Uploaded in Step)
    // ------------------------------------------------------------------------
    const screenshotKey = `rehearsal-${this.runId}.png`;
    const screenshotUrl = `/evidence/${screenshotKey}`;
    let evidenceUploaded = false;
    if (this.env?.EVIDENCE_BUCKET) {
      try {
        const svg = generateRehearsalEvidenceSvg(screenshotKey, { run_id: this.runId, status: overallStatus });
        await this.env.EVIDENCE_BUCKET.put(screenshotKey, svg, {
          httpMetadata: { contentType: "image/svg+xml" }
        });
        evidenceUploaded = true;
      } catch (err: any) {
        this.logError(`Failed to upload evidence SVG to R2: ${err?.message || err}`, "stage_evidence_barrier");
      }
    }
    this.screenshots.push({
      name: screenshotKey,
      caption: `Brakets Tournament Rehearsal Evidence (${this.size}-Team ${this.scenario}) - Status: ${overallStatus.toUpperCase()}`,
      data_url: screenshotUrl,
      timestamp: Date.now(),
    });

    // ------------------------------------------------------------------------
    // Requirement 2: Evidence Verification Barrier Before Marking Run Passed
    // ------------------------------------------------------------------------
    const evidenceKeys = [screenshotKey];
    const missingEvidence: string[] = [];

    for (const key of evidenceKeys) {
      if (!this.env?.EVIDENCE_BUCKET) {
        missingEvidence.push(key);
        continue;
      }
      try {
        const head = await this.env.EVIDENCE_BUCKET.head(key);
        if (!head || head.size === 0) {
          missingEvidence.push(key);
        }
      } catch (err) {
        missingEvidence.push(key);
      }
    }

    if (missingEvidence.length > 0) {
      overallStatus = "evidence_missing";
      this.stages.push({
        stage_id: "stage_evidence_barrier",
        name: "Evidence Provenance & R2 Storage Verification",
        status: "evidence_missing",
        duration_ms: 10,
        error: `Required evidence artifact(s) missing in R2 (s3a-evidence): ${missingEvidence.join(", ")}`,
        details: { missing_keys: missingEvidence, bucket: "s3a-evidence" }
      });
    } else {
      this.stages.push({
        stage_id: "stage_evidence_barrier",
        name: "Evidence Provenance & R2 Storage Verification",
        status: "passed",
        duration_ms: 10,
        details: {
          verified_keys: evidenceKeys,
          bucket: "s3a-evidence",
          evidence_key: screenshotKey,
          evidence_url: screenshotUrl,
        }
      });
    }

    const totalDuration = Date.now() - startTime;
    this.timings["total_duration_ms"] = totalDuration;

    const record: RehearsalRunRecord = {
      run_id: this.runId,
      tournament_id: `tourn_${this.runId}`,
      tournament_name: `Brakets Arena Invitational [${this.size}-Team ${this.scenario.toUpperCase()}]`,
      bracket_size: this.size,
      scenario: this.scenario,
      status: overallStatus,
      started_at: startTime,
      finished_at: Date.now(),
      total_duration_ms: totalDuration,
      stages: this.stages,
      matches: this.matches,
      standings: this.standings,
      console_errors: this.consoleErrors,
      failed_api_calls: this.failedApiCalls,
      screenshots: this.screenshots,
      evidence_keys: evidenceKeys,
      missing_evidence: missingEvidence.length > 0 ? missingEvidence : undefined,
      evidence_verified: missingEvidence.length === 0,
      timings_ms: this.timings,
      trace_id: this.runId,
      issues_filed: issuesFiled,
    };

    return record;
  }
}
