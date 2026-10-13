import { BracketBoardClient } from "../tournaments/[id]/bracket/BracketBoardClient";
import { FIXTURE_TOURNAMENT } from "@/fixtures/bracket-fixtures";

export default function BracketPage() {
  return (
    <main className="page page--bracket">
      <BracketBoardClient
        tournamentId={FIXTURE_TOURNAMENT.id}
        tournamentName={FIXTURE_TOURNAMENT.name}
        tournamentTitle={FIXTURE_TOURNAMENT.title}
        mode="organize"
      />
    </main>
  );
}
