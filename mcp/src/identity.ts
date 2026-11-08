import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";
export class CommunicationError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}
const key = process.env.COMMUNICATION_ID_KEY || randomBytes(32).toString("hex");
export function resourceId(
  account: string,
  kind: string,
  native: string,
  context: Record<string, string> = {},
): string {
  const body = Buffer.from(
    JSON.stringify({ account, kind, native, context }),
  ).toString("base64url");
  return (
    body + "." + createHmac("sha256", key).update(body).digest("base64url")
  );
}
export function resource(
  account: string,
  kind: string,
  id: string,
): { native: string; context: Record<string, string> } {
  const [body, sig, extra] = id.split(".");
  const expected = createHmac("sha256", key)
    .update(body || "")
    .digest();
  const got = Buffer.from(sig || "", "base64url");
  if (
    !body ||
    extra ||
    got.length !== expected.length ||
    !timingSafeEqual(got, expected)
  )
    throw new CommunicationError(
      "INVALID_RESOURCE",
      "Invalid resource handle; list resources again",
    );
  let value;
  try {
    value = JSON.parse(Buffer.from(body, "base64url").toString());
  } catch {
    throw new CommunicationError("INVALID_RESOURCE", "Invalid resource handle");
  }
  if (
    value.account !== account ||
    value.kind !== kind ||
    typeof value.native !== "string"
  )
    throw new CommunicationError(
      "ACCOUNT_MISMATCH",
      "Resource belongs to another account or resource type",
    );
  return value;
}
export function unsupported(provider: string, operation: string): never {
  throw new CommunicationError(
    "UNSUPPORTED_CAPABILITY",
    provider + " does not support " + operation,
  );
}
