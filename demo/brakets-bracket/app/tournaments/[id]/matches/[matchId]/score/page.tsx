import React from "react";
import Link from "next/link";
import { MatchScoreForm } from "@/components/MatchScoreForm";

export default async function MatchScorePage({
  params,
}: {
  params: Promise<{ id: string; matchId: string }>;
}) {
  const { id, matchId } = await params;

  return (
    <div style={{ padding: "2rem", maxWidth: "800px", margin: "0 auto", color: "#c9d1d9" }}>
      <div style={{ marginBottom: "1.5rem" }}>
        <Link
          href={`/tournaments/${id}/bracket`}
          style={{ color: "#58a6ff", fontSize: "13px", textDecoration: "none" }}
        >
          &larr; Back to Tournament Bracket
        </Link>
        <h1 style={{ fontSize: "1.5rem", fontWeight: 700, color: "#f0f6fc", marginTop: "0.5rem" }}>
          Match Score Entry
        </h1>
        <p style={{ color: "#8b949e", fontSize: "13px" }}>
          Tournament ID: <code style={{ color: "#58a6ff" }}>{id}</code> &bull; Match ID:{" "}
          <code style={{ color: "#58a6ff" }}>{matchId}</code>
        </p>
      </div>

      <div style={{ background: "#0d1117", border: "1px solid #30363d", borderRadius: "8px", padding: "2rem" }}>
        <MatchScoreForm tournamentId={id} matchId={matchId} team1Name="CyberViper" team2Name="ShadowPulse" />
      </div>
    </div>
  );
}
