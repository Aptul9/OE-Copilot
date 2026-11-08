// Local bootstrap only. Agent never invokes gweb/gws or reads the PC profile.
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";

function runPlaywriter(args: string[]): string {
  const env = { ...process.env, PLAYWRITER_TELEMETRY: "0", OE_PLAYWRITER_ARGS: JSON.stringify(args) };
  const result = process.platform === "win32"
    ? spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", "$a = ConvertFrom-Json $env:OE_PLAYWRITER_ARGS; & playwriter @a"], { env, encoding: "utf8", windowsHide: true, timeout: 360_000, maxBuffer: 2 * 1024 * 1024 })
    : spawnSync("playwriter", args, { env, encoding: "utf8", timeout: 360_000, maxBuffer: 2 * 1024 * 1024 });
  // Playwriter can exit nonzero after completing a Windows call. The saved JSON is the receipt.
  return result.stdout ?? "";
}

export function captureGoogleSession(browserKey: string, run: (args: string[]) => string = runPlaywriter): unknown {
  if (!/^install:[A-Za-z0-9:._-]{1,150}$/.test(browserKey)) throw new Error("browser must be an installed profile key from Playwriter");
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "oe-google-login-"));
  const output = path.join(directory, "session.json");
  const script = path.join(directory, "capture.js");
  let session: string | undefined;
  try {
    const created = run(["session", "new", "--browser", browserKey, "--tab-group", "oegoogle"]);
    session = /Session (\d+) created/.exec(created)?.[1];
    if (!session) throw new Error("Playwriter profile unavailable; open the signed-in profile and enable its extension");
    fs.writeFileSync(script, `const out = ${JSON.stringify(output)};
state.page = await context.newPage();
try {
  await state.page.goto('https://chat.google.com/', { waitUntil: 'domcontentloaded' });
  await state.page.waitForURL(u => u.hostname === 'chat.google.com', { timeout: 300000 });
  console.log(await snapshot({ page: state.page, search: /Google Account:/ }));
  console.log(await getLatestLogs({ page: state.page, sinceLastCall: true }));
  const cdp = await getCDPSession({ page: state.page });
  const { cookies } = await cdp.send('Network.getCookies', { urls: ['https://chat.google.com/', 'https://accounts.google.com/', 'https://www.google.com/'] });
  const user_agent = await state.page.evaluate(() => navigator.userAgent);
  require('node:fs').writeFileSync(out, JSON.stringify({ user_agent, cookies: cookies.map(c => ({ name: c.name, value: c.value, domain: c.domain, path: c.path, expires: c.expires })) }), { mode: 0o600 });
} finally { await state.page.close(); }
`, { mode: 0o600 });
    run(["-s", session, "--timeout", "330000", "-f", script]);
    if (!fs.existsSync(output)) throw new Error("Google Chat sign-in did not complete in selected browser profile");
    return readGoogleInput(output);
  } finally {
    if (session) run(["session", "delete", session]);
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

export function readGoogleInput(file: string, encryptedGweb = false): unknown {
  let raw: Buffer;
  if (encryptedGweb) {
    if (process.platform !== "win32") throw new Error("gweb DPAPI import requires Windows current user");
    const script = "Add-Type -AssemblyName System.Security; $b = [Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($env:OE_GOOGLE_INPUT), $null, [Security.Cryptography.DataProtectionScope]::CurrentUser); [Console]::OpenStandardOutput().Write($b,0,$b.Length)";
    try { raw = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { env: { ...process.env, OE_GOOGLE_INPUT: path.resolve(file) }, windowsHide: true, timeout: 20_000, maxBuffer: 2 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"] }); }
    catch { throw new Error("gweb DPAPI session could not be read"); }
  } else raw = fs.readFileSync(file === "-" ? 0 : file);
  try { return JSON.parse(raw.toString("utf8").replace(/^\uFEFF/, "")); }
  catch { throw new Error("Google input is not JSON"); }
  finally { raw.fill(0); }
}

export async function importGoogle(options: { url: string; key: string; provider: "gmail" | "gchat"; account: string; data: unknown; fetchFn?: typeof fetch }): Promise<{ account: string; provider: string }> {
  const base = new URL(options.url);
  if (base.protocol !== "https:" && !(base.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(base.hostname)) || base.username || base.password || base.search || base.hash || base.pathname !== "/") throw new Error("agent URL must be HTTPS or loopback HTTP origin");
  if (!options.key) throw new Error("API_KEY or --key-file required");
  if (!["gmail", "gchat"].includes(options.provider)) throw new Error("provider must be gmail or gchat");
  if (!/^[^\s@]+@[^\s@]+$/.test(options.account)) throw new Error("exact account email required");
  const body = { provider: options.provider, account: options.account, ...(options.provider === "gmail" ? { credentials: options.data } : { sessione: options.data }) };
  let response: Response;
  try { response = await (options.fetchFn ?? fetch)(new URL("/api/auth/completa", base).href, { method: "POST", headers: { Authorization: `Bearer ${options.key}`, "Content-Type": "application/json" }, body: JSON.stringify(body), redirect: "error", signal: AbortSignal.timeout(90_000) }); }
  catch { throw new Error("Google import transport failed"); }
  if (!response.ok) { await response.body?.cancel(); throw new Error(`Google import HTTP ${response.status}`); }
  const result = await response.json() as Record<string, unknown>;
  if (result.ok !== true || typeof result.account !== "string" || result.account.toLowerCase() !== options.account.toLowerCase() || result.provider !== options.provider) throw new Error("Google import returned invalid account result");
  return { account: result.account, provider: options.provider };
}

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { url: { type: "string" }, provider: { type: "string" }, account: { type: "string" }, file: { type: "string" }, browser: { type: "string" }, "gweb-session": { type: "boolean" }, "key-file": { type: "string" } } });
  if (!values.url || !values.account || !["gmail", "gchat"].includes(values.provider ?? "")) throw new Error("usage: node scripts/import-google.ts --url ORIGIN --provider gmail|gchat --account EMAIL --file FILE [--gweb-session] [--key-file FILE]");
  const file = values.file ?? (values["gweb-session"] && process.env.APPDATA ? path.join(process.env.APPDATA, "gweb", "session.bin") : "");
  if (values.browser && (values.file || values["gweb-session"] || values.provider !== "gchat")) throw new Error("--browser is for gchat, without --file or --gweb-session");
  if (!file && !values.browser) throw new Error("--file required (use - for stdin), or --browser PROFILE for Chat capture");
  const data = values.browser ? captureGoogleSession(values.browser) : readGoogleInput(file, values["gweb-session"]);
  const key = values["key-file"] ? fs.readFileSync(values["key-file"], "utf8").trim() : process.env.API_KEY ?? "";
  process.stdout.write(JSON.stringify(await importGoogle({ url: values.url, provider: values.provider as "gmail" | "gchat", account: values.account, data, key })) + "\n");
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch(error => { process.stderr.write((error instanceof Error ? error.message : "Google import failed") + "\n"); process.exitCode = 1; });
