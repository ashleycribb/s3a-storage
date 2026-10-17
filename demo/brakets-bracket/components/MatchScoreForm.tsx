"use client";

import React, { useState } from "react";

export interface MatchScoreFormProps {
  tournamentId: string;
  matchId: string;
  team1Name?: string;
  team2Name?: string;
  onSubmitSuccess?: (result: any) => void;
}

export function MatchScoreForm({
  tournamentId,
  matchId,
  team1Name = "Team 1",
  team2Name = "Team 2",
  onSubmitSuccess,
}: MatchScoreFormProps) {
  const [score1, setScore1] = useState<number | "">("");
  const [score2, setScore2] = useState<number | "">("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const isValid =
    typeof score1 === "number" &&
    typeof score2 === "number" &&
    score1 >= 0 &&
    score2 >= 0 &&
    score1 !== score2; // Reject draws in single-elimination

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!isValid || submitting) return;

    setSubmitting(true);
    setError(null);

    try {
      const res = await fetch(`/api/tournaments/${tournamentId}/report`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          lobbyId: matchId,
          score: { [team1Name]: score1, [team2Name]: score2 },
          standings: score1 > score2 ? [team1Name, team2Name] : [team2Name, team1Name],
          role: "player",
        }),
      });

      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data.message || data.error || `HTTP ${res.status}`);
      }

      const result = await res.json();
      setSuccess(true);
      if (onSubmitSuccess) onSubmitSuccess(result);
    } catch (err: any) {
      setError(err.message || "Failed to submit match score");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form
      id="form-match-score"
      onSubmit={handleSubmit}
      style={{
        background: "#161b22",
        border: "1px solid #30363d",
        borderRadius: "8px",
        padding: "1.5rem",
        maxWidth: "480px",
        margin: "0 auto",
        color: "#c9d1d9",
      }}
    >
      <h3 style={{ fontSize: "1.1rem", fontWeight: 600, color: "#f0f6fc", marginBottom: "1rem" }}>
        Report Match Score
      </h3>

      <div style={{ display: "grid", gridTemplateColumns: "1fr auto 1fr", gap: "1rem", alignItems: "center", marginBottom: "1rem" }}>
        <div>
          <label style={{ display: "block", fontSize: "12px", color: "#8b949e", marginBottom: "0.25rem" }}>
            {team1Name}
          </label>
          <input
            id="input-team1-score"
            type="number"
            min="0"
            value={score1}
            onChange={(e) => setScore1(e.target.value === "" ? "" : Number(e.target.value))}
            placeholder="0"
            style={{
              width: "100%",
              padding: "8px 12px",
              background: "#0d1117",
              border: "1px solid #30363d",
              borderRadius: "6px",
              color: "#f0f6fc",
              fontSize: "16px",
              textAlign: "center",
            }}
          />
        </div>

        <span style={{ fontSize: "1.2rem", fontWeight: 600, color: "#8b949e" }}>VS</span>

        <div>
          <label style={{ display: "block", fontSize: "12px", color: "#8b949e", marginBottom: "0.25rem" }}>
            {team2Name}
          </label>
          <input
            id="input-team2-score"
            type="number"
            min="0"
            value={score2}
            onChange={(e) => setScore2(e.target.value === "" ? "" : Number(e.target.value))}
            placeholder="0"
            style={{
              width: "100%",
              padding: "8px 12px",
              background: "#0d1117",
              border: "1px solid #30363d",
              borderRadius: "6px",
              color: "#f0f6fc",
              fontSize: "16px",
              textAlign: "center",
            }}
          />
        </div>
      </div>

      {score1 !== "" && score2 !== "" && score1 === score2 && (
        <div style={{ color: "#f85149", fontSize: "12px", marginBottom: "1rem" }}>
          Single-elimination matches cannot end in a draw. Sudden-death tiebreaker required.
        </div>
      )}

      {error && (
        <div style={{ color: "#f85149", fontSize: "12px", marginBottom: "1rem" }}>
          {error}
        </div>
      )}

      {success && (
        <div style={{ color: "#3fb950", fontSize: "12px", marginBottom: "1rem" }}>
          Score successfully reported and official tournament bracket advanced!
        </div>
      )}

      <button
        id="btn-submit-match-score"
        type="submit"
        disabled={!isValid || submitting}
        style={{
          width: "100%",
          padding: "10px 16px",
          background: isValid ? "#238636" : "#21262d",
          color: isValid ? "#ffffff" : "#8b949e",
          border: "1px solid " + (isValid ? "rgba(240,246,252,0.1)" : "#30363d"),
          borderRadius: "6px",
          fontWeight: 600,
          fontSize: "14px",
          cursor: isValid ? "pointer" : "not-allowed",
          pointerEvents: "auto", // Cleanly enables pointer events (resolves fp_brokenco_0941ea36)
          transition: "background 0.15s ease",
        }}
      >
        {submitting ? "Submitting Official Score..." : "Submit Official Match Score"}
      </button>
    </form>
  );
}
