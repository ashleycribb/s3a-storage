// Response body for the unauthenticated GET /project/:id/control.

export interface StoredControlState {
  paused: boolean;
  paused_by?: string;
  paused_reason?: string;
  paused_at?: number;
  scheduler_status?: string;
}

// Only `paused` and `status` are public (security hardening, c2b3b68). Who paused, why and when
// are audit data: they stay in storage, in the authenticated POST response and in /status.
export function publicControlView(s: StoredControlState, _nowMs: number): { paused: boolean; status: string } {
  return {
    paused: s.paused,
    status: s.scheduler_status || (s.paused ? "paused" : "idle"),
  };
}
