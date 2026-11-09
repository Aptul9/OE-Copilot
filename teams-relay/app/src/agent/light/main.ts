// TeamsRelay light agent of one slot: a Teams account with no browser of its own. It reads the chats and the Activity
// feed through the Teams services, sends the push notifications and runs the commands of the web app, on the sign-in
// the account's browser run captured (src/agent/jobs/light-token.ts). Built into dist/light.cjs (esbuild).
//   node light.cjs           runs the light agent (environment: src/agent/config.ts)
//   node light.cjs --host    runs the slots the supervisor sends over IPC, all in this process (src/agent/light/host.ts)
//   node light.cjs --check   loads the runtime dependencies, then exits
import Database from "better-sqlite3";
import { ConfigError } from "@/shared/env";
import { errorText, log } from "../log";
import { runHost } from "./host";
import { HOST_FLAG } from "./host-protocol";
import { startSlot } from "./slot";
import { traceSlowRequests } from "./slow-requests";

function check() {
  const db = new Database(":memory:");
  const sqlite = (db.prepare("SELECT sqlite_version() AS v").get() as { v: string }).v;
  db.close();
  log.info("light", "check ok", { sqlite });
}

async function main() {
  if (process.argv.includes("--check")) return check();
  traceSlowRequests();
  if (process.argv.includes(HOST_FLAG)) return runHost();
  for (const signal of ["SIGTERM", "SIGINT"] as const) process.on(signal, () => { /* runLight owns graceful native-worker shutdown */ setTimeout(() => process.exit(0), 4000).unref(); });
  await startSlot(process.env);
}

main().catch((e: unknown) => {
  log.warn("light", e instanceof ConfigError ? `configuration: ${e.message}` : errorText(e));
  process.exit(1);
});
