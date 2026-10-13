import { BracketBoardClient } from "../BracketBoardClient";
import { FIXTURE_TOURNAMENT } from "@/fixtures/bracket-fixtures";

export default async function TournamentBracketSharePage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ embed?: string | string[] }>;
}) {
  const { id } = await params;
  const sp = await searchParams;
  const rawEmbed = Array.isArray(sp.embed) ? sp.embed[0] : sp.embed;
  const embed = rawEmbed === "1" || rawEmbed === "true";

  return (
    <main className="page page--bracket">
      <BracketBoardClient
        tournamentId={id || FIXTURE_TOURNAMENT.id}
        tournamentName={FIXTURE_TOURNAMENT.name}
        tournamentTitle={FIXTURE_TOURNAMENT.title}
        mode="share"
        embed={embed}
      />
    </main>
  );
}
