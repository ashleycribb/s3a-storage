import { FIXTURE_TOURNAMENT, INITIAL_BRACKET_SNAPSHOT, FIXTURE_WALK_UP } from "@/fixtures/bracket-fixtures";
import type { FinalsLikeSnapshot } from "./bracket-board";

interface MockReportEvent {
  id: string;
  lobbyId: string;
  reportedBy: string;
  role: string;
  status: string;
  createdAt: string;
  notes?: string;
}

class MockApiStore {
  private tournament = { ...FIXTURE_TOURNAMENT };
  private snapshot: FinalsLikeSnapshot = JSON.parse(JSON.stringify(INITIAL_BRACKET_SNAPSHOT));
  private auditEvents: MockReportEvent[] = [];
  private conflictLobbyIds: string[] = [];
  private walkUp = [...FIXTURE_WALK_UP];

  getTournament(id: string) {
    if (this.tournament.id === id || id === "fixture" || id === "demo") {
      return this.tournament;
    }
    return this.tournament;
  }

  getBracket(tournamentId: string): FinalsLikeSnapshot {
    return this.snapshot;
  }

  reassignTeam(
    tournamentId: string,
    from: { lobbyId: string; teamId: string },
    to: { lobbyId: string; teamId?: string; slotIndex?: number }
  ): FinalsLikeSnapshot {
    const lobbies = this.snapshot.lobbies;
    const fromLobby = lobbies.find((l) => l.id === from.lobbyId);
    const toLobby = lobbies.find((l) => l.id === to.lobbyId);

    if (fromLobby && toLobby) {
      if (to.teamId) {
        // Swap
        const fromIdx = fromLobby.teamIds.indexOf(from.teamId);
        const toIdx = toLobby.teamIds.indexOf(to.teamId);
        if (fromIdx !== -1 && toIdx !== -1) {
          fromLobby.teamIds[fromIdx] = to.teamId;
          toLobby.teamIds[toIdx] = from.teamId;
        }
      } else {
        // Move to empty
        const fromIdx = fromLobby.teamIds.indexOf(from.teamId);
        if (fromIdx !== -1) {
          fromLobby.teamIds.splice(fromIdx, 1);
          if (typeof to.slotIndex === "number" && to.slotIndex <= toLobby.teamIds.length) {
            toLobby.teamIds.splice(to.slotIndex, 0, from.teamId);
          } else {
            toLobby.teamIds.push(from.teamId);
          }
        }
      }
      this.snapshot.seq = (this.snapshot.seq ?? 0) + 1;
    }

    return this.snapshot;
  }

  seedBracket(tournamentId: string, teamNames: string[]): FinalsLikeSnapshot {
    const teams = teamNames.slice(0, 8).map((name, i) => ({
      id: `team_${i + 1}`,
      name,
      seed: i + 1,
    }));

    this.snapshot.teams = teams;
    this.snapshot.lobbies[0].teamIds = teams.slice(0, 4).map((t) => t.id);
    this.snapshot.lobbies[1].teamIds = teams.slice(4, 8).map((t) => t.id);
    this.snapshot.lobbies[0].status = "live";
    this.snapshot.lobbies[1].status = "live";
    this.snapshot.seq = (this.snapshot.seq ?? 0) + 1;

    return this.snapshot;
  }

  reportLobby(
    tournamentId: string,
    lobbyId: string,
    standings: string[],
    role: string
  ): { reportStatus: string } {
    const lobby = this.snapshot.lobbies.find((l) => l.id === lobbyId);
    if (!lobby) {
      return { reportStatus: "error" };
    }

    lobby.standings = standings;
    lobby.status = "final";

    const evt: MockReportEvent = {
      id: `evt_${Date.now()}`,
      lobbyId,
      reportedBy: "organizer_demo",
      role,
      status: "accepted",
      createdAt: new Date().toISOString(),
    };
    this.auditEvents.push(evt);

    return { reportStatus: "accepted" };
  }

  resolveReport(
    tournamentId: string,
    lobbyId: string,
    note?: string
  ): { success: boolean; alreadyResolved?: boolean } {
    this.conflictLobbyIds = this.conflictLobbyIds.filter((id) => id !== lobbyId);
    this.auditEvents.push({
      id: `evt_res_${Date.now()}`,
      lobbyId,
      reportedBy: "supervisor_human",
      role: "admin",
      status: "resolved",
      notes: note,
      createdAt: new Date().toISOString(),
    });
    return { success: true };
  }

  getAudit(tournamentId: string) {
    return {
      events: this.auditEvents,
      conflictLobbyIds: this.conflictLobbyIds,
      evidenceCountsByLobbyId: {},
    };
  }

  getEvidence(tournamentId: string) {
    return { items: [] };
  }

  getWalkUp(tournamentId: string) {
    return { entries: this.walkUp };
  }

  getReady(tournamentId: string) {
    return { ready: [] };
  }

  getStreamPresence(tournamentId: string) {
    return { configured: false, players: [] };
  }
}

export const mockStore = new MockApiStore();

// --- S3A Agent Autonomous Contribution: Role-Aware Auth (Issue #3) ---
export type PlatformRole = "player" | "coach" | "team_manager" | "streamer" | "organizer";

export interface UserRoleRecord {
  userId: string;
  handle: string;
  primaryRole: PlatformRole;
  permissions: {
    canCreateTournament: boolean;
    canReportScores: boolean;
    canManageRoster: boolean;
    canStreamMatch: boolean;
  };
}

export function evaluatePlatformPermissions(role: PlatformRole) {
  return {
    canCreateTournament: role === "organizer",
    canReportScores: role === "organizer" || role === "player" || role === "coach",
    canManageRoster: role === "team_manager" || role === "coach",
    canStreamMatch: role === "streamer" || role === "player",
  };
}
