import { BracketBoardClient } from "../../tournaments/[id]/bracket/BracketBoardClient";
import { FIXTURE_TOURNAMENT } from "@/fixtures/bracket-fixtures";

export default async function BracketSharePage({
  searchParams,
}: {
  searchParams: Promise<{ embed?: string | string[] }>;
}) {
  const sp = await searchParams;
  const rawEmbed = Array.isArray(sp.embed) ? sp.embed[0] : sp.embed;
  const embed = rawEmbed === "1" || rawEmbed === "true";

  return (
    <main className="page page--bracket">
      <BracketBoardClient
        tournamentId={FIXTURE_TOURNAMENT.id}
        tournamentName={FIXTURE_TOURNAMENT.name}
        tournamentTitle={FIXTURE_TOURNAMENT.title}
        mode="share"
        embed={embed}
      />
    </main>
  );
}
