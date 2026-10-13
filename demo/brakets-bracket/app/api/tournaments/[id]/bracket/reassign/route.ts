import { NextResponse } from "next/server";
import { mockStore } from "@/lib/mock-api";

export async function POST(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  try {
    const body = await request.json();
    const { from, to } = body;
    const snapshot = mockStore.reassignTeam(id, from, to);
    return NextResponse.json(snapshot);
  } catch (err) {
    return NextResponse.json({ error: "invalid_request" }, { status: 400 });
  }
}
