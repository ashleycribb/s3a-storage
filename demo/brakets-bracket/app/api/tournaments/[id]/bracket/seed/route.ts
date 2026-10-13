import { NextResponse } from "next/server";
import { mockStore } from "@/lib/mock-api";

export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  try {
    const body = await request.json();
    const { teams } = body;
    if (!Array.isArray(teams) || teams.length !== 8) {
      return NextResponse.json({ error: "eight_team_names_required" }, { status: 400 });
    }
    const snapshot = mockStore.seedBracket(id, teams);
    return NextResponse.json(snapshot);
  } catch (err) {
    return NextResponse.json({ error: "invalid_request" }, { status: 400 });
  }
}
