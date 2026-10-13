import { NextResponse } from "next/server";
import { mockStore } from "@/lib/mock-api";

export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  try {
    const body = await request.json();
    const { lobbyId, standings, role } = body;
    const res = mockStore.reportLobby(id, lobbyId, standings, role || "to");
    return NextResponse.json(res);
  } catch (err) {
    return NextResponse.json({ error: "invalid_report_request" }, { status: 400 });
  }
}
