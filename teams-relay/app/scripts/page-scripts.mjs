// The page scripts of the agent (src/agent/teams/scripts) as the Rust agent sends them: each function as source text,
// the text Playwright sends for the Node agent (esbuild with the options of the agent bundle), and the selectors and
// texts they take (src/agent/teams/selectors.ts), a RegExp as {"$re": [source, flags]}.
//   node scripts/page-scripts.mjs   writes webapp-rs/src/agent/browser/page-scripts.json
// test/agent/page-scripts-json.test.ts fails while that file is not what this makes of the sources.
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { build } from "esbuild";

const APP = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const OUT = path.join(APP, "webapp-rs/src/agent/browser/page-scripts.json");

const MODULES = ["activity", "calls", "chat-list", "compose", "conversation", "media", "members", "mentions", "message-actions", "page-state", "sign-in", "tokens"];
const SELECTORS = ["SEL", "TEXTS", "ACTIONS", "BAR_REACTIONS", "PICKER_REACTIONS", "TEAMS_HOSTS", "LOGIN_HOSTS"];
const camel = (m) => m.replace(/-(\w)/g, (_, c) => c.toUpperCase());
// a helper of the bundler does not exist in the page (the --check of the agent bundle refuses it too)
const HELPER = /\b__(name|async|awaiter|spreadValues|spreadProps|objRest|publicField|require|toESM)\b/;

const encode = (v) =>
  v instanceof RegExp
    ? { $re: [v.source, v.flags] }
    : Array.isArray(v)
      ? v.map(encode)
      : v && typeof v === "object"
        ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, encode(x)]))
        : v;

export async function pageScriptsJson() {
  const contents = [
    ...MODULES.map((m) => `export * as ${camel(m)} from "./src/agent/teams/scripts/${m}";`),
    `export { ${SELECTORS.join(", ")} } from "./src/agent/teams/selectors";`,
  ].join("\n");
  const r = await build({ stdin: { contents, resolveDir: APP, loader: "ts" }, absWorkingDir: APP, bundle: true, platform: "node", target: "node24", format: "cjs", write: false, logLevel: "silent" });
  const mod = { exports: {} };
  new Function("module", "exports", "require", r.outputFiles[0].text)(mod, mod.exports, createRequire(import.meta.url));
  const m = mod.exports;
  const scripts = {};
  for (const name of MODULES) {
    for (const [fn, value] of Object.entries(m[camel(name)]).sort(([a], [b]) => a.localeCompare(b))) {
      if (typeof value !== "function") continue;
      const source = value.toString();
      if (HELPER.test(source)) throw new Error(`page script ${name}.${fn} calls a bundler helper, which does not exist in the page`);
      new Function(`return (${source})`);
      scripts[`${name}.${fn}`] = source;
    }
  }
  const out = { _generated: "by scripts/page-scripts.mjs from src/agent/teams/scripts and src/agent/teams/selectors.ts: do not edit", scripts };
  for (const s of SELECTORS) out[camel(s.toLowerCase())] = encode(m[s]);
  return `${JSON.stringify(out, null, 1)}\n`;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  fs.mkdirSync(path.dirname(OUT), { recursive: true });
  fs.writeFileSync(OUT, await pageScriptsJson());
  console.log(`page scripts written: ${path.relative(APP, OUT)}`);
}
