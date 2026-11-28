import { NextResponse } from "next/server";

export async function GET(req: Request) {
  return NextResponse.json([]);
}

export const POST = async (req: Request) => NextResponse.json({});

export function helper() {}
