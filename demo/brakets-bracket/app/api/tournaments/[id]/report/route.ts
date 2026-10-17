import { NextResponse } from "next/server";
import { mockStore } from "@/lib/mock-api";
import { validateMatchScore } from "../../../../../packages/core/src/tournament";

export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  try {
    const body = await request.json();
    const { lobbyId, standings, score, role } = body;

    // Validate score to prevent unhandled draw (resolves fp_unhandle_09367ddd)
    if (score && typeof score === "object") {
      const validation = validateMatchScore("single_elim", score);
      if (!validation.valid) {
        return NextResponse.json(
          { error: "draw_not_permitted", message: validation.error },
          { status: 422 }
        );
      }
    }

    const res = mockStore.reportLobby(id, lobbyId, standings, role || "to");
    return NextResponse.json(res);
  } catch (err) {
    return NextResponse.json({ error: "invalid_report_request" }, { status: 400 });
  }
}
