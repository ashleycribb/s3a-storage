import { NextResponse } from "next/server";
import { mockStore } from "@/lib/mock-api";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const snapshot = mockStore.getBracket(id);
  return NextResponse.json(snapshot);
}
