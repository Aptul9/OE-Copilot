import fs from "node:fs";
import { expect, it } from "vitest";
import { OUT, pageScriptsJson } from "../../scripts/page-scripts.mjs";

// The Rust agent sends the page scripts of webapp-rs/src/agent/browser/page-scripts.json: regenerated from the sources
it("the page scripts of the Rust agent are those of the sources (node scripts/page-scripts.mjs)", async () => {
  expect(fs.readFileSync(OUT, "utf8")).toBe(await pageScriptsJson());
}, 30_000);
