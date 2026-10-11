import { auth } from "@/lib/auth";

export const dynamic = "force-dynamic";

export function GET(req: Request) {
  return auth().handler(req);
}

export function POST(req: Request) {
  return auth().handler(req);
}
