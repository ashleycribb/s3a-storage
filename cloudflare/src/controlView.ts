// Response body for the unauthenticated GET /project/:id/control.

export interface StoredControlState {
  paused: boolean;
  paused_by?: string;
  paused_reason?: string;
  paused_at?: number;
  scheduler_status?: string;
}

export function publicControlView(s: StoredControlState, nowMs: number): Record<string, unknown> {
  return {
    paused: s.paused,
    paused_by: s.paused_by || "Supervisor (Ashley)",
    paused_reason: s.paused_reason || (s.paused ? "Swarm paused by supervisor" : "Operating normally"),
    paused_at: s.paused_at || nowMs,
    status: s.scheduler_status || (s.paused ? "paused" : "idle"),
  };
}
