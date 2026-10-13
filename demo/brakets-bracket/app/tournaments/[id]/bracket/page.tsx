import { BracketBoardClient } from "./BracketBoardClient";
import { FIXTURE_TOURNAMENT } from "@/fixtures/bracket-fixtures";

export default async function TournamentBracketPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  return (
    <main className="page page--bracket">
      <BracketBoardClient
        tournamentId={id || FIXTURE_TOURNAMENT.id}
        tournamentName={FIXTURE_TOURNAMENT.name}
        tournamentTitle={FIXTURE_TOURNAMENT.title}
        mode="organize"
      />
    </main>
  );
}
