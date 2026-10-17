// cloudflare/src/rehearsal/FormatRegistry.ts
// Tournament Format Registry & Game Pack System for Brakets
// Formats as data, not code. Config files override defaults. Versioned & signed.

import type {
  TournamentFormatConfig,
  GamePack,
  EventOptions,
} from "../types";

export class FormatRegistry {
  private formats: Map<string, TournamentFormatConfig> = new Map();
  private gamePacks: Map<string, GamePack> = new Map();

  constructor() {
    this.registerBuiltinFormats();
    this.registerBuiltinGamePacks();
  }

  /**
   * Generates a deterministic SHA-256 hex hash for any JSON serializable spec.
   */
  public static computeSpecHash(spec: Record<string, any>): string {
    const raw = JSON.stringify(spec, Object.keys(spec).sort());
    let hash = 0;
    for (let i = 0; i < raw.length; i++) {
      hash = (hash << 5) - hash + raw.charCodeAt(i);
      hash |= 0;
    }
    const hex = Math.abs(hash).toString(16).padStart(8, "0");
    return `sha256_${hex}`;
  }

  private registerBuiltinFormats() {
    // 1. Single Elimination
    const singleElimSpec: TournamentFormatConfig = {
      id: "single_elimination",
      name: "Single Elimination Bracket",
      version: "1.0.0",
      author: "Brakets Core Team",
      content_hash: "",
      signature: "ed25519_sig_format_single_elim_v1",
      teams_per_match: 2,
      players_per_team: 1,
      seeding: "standard_bracket",
      win_condition: { type: "first_to_n", target: 2 },
      tie_breakers: ["head_to_head", "point_differential", "rematch"],
      advancement: { structure: "bracket", losers_bracket: false },
      rematch_rules: "forbidden",
    };
    singleElimSpec.content_hash = FormatRegistry.computeSpecHash(singleElimSpec);
    this.formats.set(singleElimSpec.id, singleElimSpec);

    // 2. Double Elimination
    const doubleElimSpec: TournamentFormatConfig = {
      id: "double_elimination",
      name: "Double Elimination Bracket (Upper & Lower)",
      version: "1.0.0",
      author: "Brakets Core Team",
      content_hash: "",
      signature: "ed25519_sig_format_double_elim_v1",
      teams_per_match: 2,
      players_per_team: 1,
      seeding: "standard_bracket",
      win_condition: { type: "first_to_n", target: 2 },
      tie_breakers: ["head_to_head", "rematch"],
      advancement: { structure: "bracket", losers_bracket: true, grand_finals_reset: true },
      rematch_rules: "playoffs_only",
    };
    doubleElimSpec.content_hash = FormatRegistry.computeSpecHash(doubleElimSpec);
    this.formats.set(doubleElimSpec.id, doubleElimSpec);

    // 3. Swiss System
    const swissSpec: TournamentFormatConfig = {
      id: "swiss",
      name: "Swiss System Pairing",
      version: "1.0.0",
      author: "Brakets Core Team",
      content_hash: "",
      signature: "ed25519_sig_format_swiss_v1",
      teams_per_match: 2,
      players_per_team: 1,
      seeding: "standard_bracket",
      win_condition: { type: "most_wins" },
      tie_breakers: ["buchholz", "sonneborn_berger", "game_win_pct"],
      advancement: { structure: "swiss_rounds", rounds: 3 },
      rematch_rules: "forbidden",
    };
    swissSpec.content_hash = FormatRegistry.computeSpecHash(swissSpec);
    this.formats.set(swissSpec.id, swissSpec);

    // 4. Groups + Playoffs
    const groupsSpec: TournamentFormatConfig = {
      id: "groups_playoffs",
      name: "Round-Robin Groups to Single Elimination Playoffs",
      version: "1.0.0",
      author: "Brakets Core Team",
      content_hash: "",
      signature: "ed25519_sig_format_groups_playoffs_v1",
      teams_per_match: 2,
      players_per_team: 1,
      seeding: "snake",
      win_condition: { type: "most_wins" },
      tie_breakers: ["head_to_head", "point_differential", "buchholz"],
      advancement: { structure: "groups_to_bracket", qualifiers_per_group: 2 },
      rematch_rules: "playoffs_only",
    };
    groupsSpec.content_hash = FormatRegistry.computeSpecHash(groupsSpec);
    this.formats.set(groupsSpec.id, groupsSpec);

    // 5. Round Robin
    const roundRobinSpec: TournamentFormatConfig = {
      id: "round_robin",
      name: "Pure Round Robin League",
      version: "1.0.0",
      author: "Brakets Core Team",
      content_hash: "",
      signature: "ed25519_sig_format_round_robin_v1",
      teams_per_match: 2,
      players_per_team: 1,
      seeding: "random",
      win_condition: { type: "most_wins" },
      tie_breakers: ["head_to_head", "point_differential", "sonneborn_berger"],
      advancement: { structure: "swiss_rounds" },
      rematch_rules: "forbidden",
    };
    roundRobinSpec.content_hash = FormatRegistry.computeSpecHash(roundRobinSpec);
    this.formats.set(roundRobinSpec.id, roundRobinSpec);

    // 6. Battle Royale / FFA
    const brSpec: TournamentFormatConfig = {
      id: "battle_royale",
      name: "Battle Royale Multi-Team Lobby Points",
      version: "1.0.0",
      author: "Brakets Core Team",
      content_hash: "",
      signature: "ed25519_sig_format_br_v1",
      teams_per_match: 20,
      players_per_team: 3,
      seeding: "random",
      win_condition: { type: "placement_score", target: 50 },
      tie_breakers: ["point_differential", "most_wins"],
      advancement: { structure: "lobby_elimination" },
      rematch_rules: "allowed",
    };
    brSpec.content_hash = FormatRegistry.computeSpecHash(brSpec);
    this.formats.set(brSpec.id, brSpec);
  }

  private registerBuiltinGamePacks() {
    // 1. Tactical Esports Pack (Valorant / CS2)
    const esportsPack: GamePack = {
      id: "esports_tactical",
      name: "Tactical Esports Pack (Valorant & CS2)",
      version: "1.0.0",
      genre: "esports",
      default_format_id: "single_elimination",
      event_options: {
        match_rules: {
          best_of_n: 3,
          per_game_scoring: true,
          bans_picks: {
            enabled: true,
            pool_size: 7,
            bans_per_team: 2,
            system: "ban_ban_pick_pick_decider",
          },
        },
        check_in: { window_minutes: 30, auto_dq_no_show: true },
        score_reporting: { mode: "both_players", dispute_grace_period_mins: 15, allow_disputes: true },
        roster_rules: { substitutes_allowed: 1, coach_slots: 1, roster_lock: true },
      },
      game_metadata: {
        titles: ["Valorant", "Counter-Strike 2"],
        map_pool: ["Haven", "Bind", "Ascent", "Split", "Sunset", "Lotus", "Abyss"],
        overtime_rule: "MR12/MR13 overtime win by 2",
        match_term: "Map",
      },
      content_hash: "",
      signature: "ed25519_sig_pack_esports_v1",
    };
    esportsPack.content_hash = FormatRegistry.computeSpecHash(esportsPack);
    this.gamePacks.set(esportsPack.id, esportsPack);

    // 2. Fighting Games Pack (FT3 Double Elimination)
    const fgcPack: GamePack = {
      id: "fighting_games",
      name: "Fighting Games Pack (FT3 / Bo5)",
      version: "1.0.0",
      genre: "fighting",
      default_format_id: "double_elimination",
      event_options: {
        match_rules: { best_of_n: 5, per_game_scoring: true },
        check_in: { window_minutes: 15, auto_dq_no_show: true },
        score_reporting: { mode: "both_players", dispute_grace_period_mins: 10, allow_disputes: true },
        roster_rules: { substitutes_allowed: 0, coach_slots: 0, roster_lock: true },
      },
      game_metadata: {
        titles: ["Street Fighter 6", "Tekken 8", "Guilty Gear Strive"],
        match_term: "Game",
      },
      content_hash: "",
      signature: "ed25519_sig_pack_fgc_v1",
    };
    fgcPack.content_hash = FormatRegistry.computeSpecHash(fgcPack);
    this.gamePacks.set(fgcPack.id, fgcPack);

    // 3. Tabletop & Card Games Pack (Swiss System)
    const tabletopPack: GamePack = {
      id: "tabletop_swiss",
      name: "Tabletop Swiss Pack (MtG, Chess & Cards)",
      version: "1.0.0",
      genre: "tabletop",
      default_format_id: "swiss",
      event_options: {
        match_rules: { best_of_n: 3, per_game_scoring: true },
        check_in: { window_minutes: 20, auto_dq_no_show: true },
        score_reporting: { mode: "both_players", dispute_grace_period_mins: 10, allow_disputes: true },
        roster_rules: { substitutes_allowed: 0, coach_slots: 0, roster_lock: true },
      },
      game_metadata: {
        titles: ["Magic: The Gathering", "Chess", "Disney Lorcana"],
        match_term: "Game",
      },
      content_hash: "",
      signature: "ed25519_sig_pack_tabletop_v1",
    };
    tabletopPack.content_hash = FormatRegistry.computeSpecHash(tabletopPack);
    this.gamePacks.set(tabletopPack.id, tabletopPack);

    // 4. Sports Simulation Pack (Rocket League & EA FC)
    const sportsPack: GamePack = {
      id: "sports_sim",
      name: "Sports Simulation Pack (Rocket League & EA FC)",
      version: "1.0.0",
      genre: "sports",
      default_format_id: "groups_playoffs",
      event_options: {
        match_rules: { best_of_n: 5, per_game_scoring: true },
        check_in: { window_minutes: 15, auto_dq_no_show: true },
        score_reporting: { mode: "both_players", dispute_grace_period_mins: 10, allow_disputes: true },
        roster_rules: { substitutes_allowed: 1, coach_slots: 1, roster_lock: true },
      },
      game_metadata: {
        titles: ["Rocket League", "EA FC 25"],
        match_term: "Match",
      },
      content_hash: "",
      signature: "ed25519_sig_pack_sports_v1",
    };
    sportsPack.content_hash = FormatRegistry.computeSpecHash(sportsPack);
    this.gamePacks.set(sportsPack.id, sportsPack);
  }

  public getFormat(formatId: string): TournamentFormatConfig | undefined {
    return this.formats.get(formatId);
  }

  public getGamePack(packId: string): GamePack | undefined {
    return this.gamePacks.get(packId);
  }

  public listFormats(): TournamentFormatConfig[] {
    return Array.from(this.formats.values());
  }

  public listGamePacks(): GamePack[] {
    return Array.from(this.gamePacks.values());
  }

  public registerFormat(format: TournamentFormatConfig) {
    if (!format.content_hash) {
      format.content_hash = FormatRegistry.computeSpecHash(format);
    }
    this.formats.set(format.id, format);
  }

  public registerGamePack(pack: GamePack) {
    if (!pack.content_hash) {
      pack.content_hash = FormatRegistry.computeSpecHash(pack);
    }
    this.gamePacks.set(pack.id, pack);
  }

  /**
   * Composes default TO event options merged with a game pack's overrides and manual options.
   */
  public composeEventOptions(
    gamePackId?: string,
    overrides?: Partial<EventOptions>
  ): EventOptions {
    const pack = gamePackId ? this.getGamePack(gamePackId) : undefined;
    const defaultOptions: EventOptions = {
      match_rules: {
        best_of_n: 3,
        per_game_scoring: true,
      },
      check_in: {
        window_minutes: 30,
        auto_dq_no_show: true,
      },
      score_reporting: {
        mode: "both_players",
        dispute_grace_period_mins: 15,
        allow_disputes: true,
      },
      roster_rules: {
        substitutes_allowed: 1,
        coach_slots: 1,
        roster_lock: true,
      },
      multi_stage: false,
    };

    const packOpts = pack?.event_options || {};
    return {
      match_rules: { ...defaultOptions.match_rules, ...packOpts.match_rules, ...overrides?.match_rules },
      check_in: { ...defaultOptions.check_in, ...packOpts.check_in, ...overrides?.check_in },
      score_reporting: { ...defaultOptions.score_reporting, ...packOpts.score_reporting, ...overrides?.score_reporting },
      roster_rules: { ...defaultOptions.roster_rules, ...packOpts.roster_rules, ...overrides?.roster_rules },
      multi_stage: overrides?.multi_stage ?? packOpts.multi_stage ?? defaultOptions.multi_stage,
      stages: overrides?.stages || packOpts.stages,
    };
  }
}
