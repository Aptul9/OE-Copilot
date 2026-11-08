import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, expect, it } from "vitest";
import { SignIns } from "../src/signin.ts";

const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0))
    fs.rmSync(dir, { recursive: true, force: true });
});
function store() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "signin-"));
  dirs.push(dir);
  return path.join(dir, "signin.json");
}
it("persists one address-bound job and preserves its id on repeated start", () => {
  const file = store();
  const first = new SignIns(file).start(" USER@EXAMPLE.COM ", ["mail", "chat"]);
  expect(new SignIns(file).start("user@example.com", ["mail", "chat"]).id).toBe(
    first.id,
  );
  expect(new SignIns(file).list()[0]).toMatchObject({
    account: "user@example.com",
    capabilities: ["mail", "chat"],
    phase: "queued",
  });
});
it("rejects unknown capabilities, addresses and uncorrelated session results", () => {
  const jobs = new SignIns(store());
  expect(() => jobs.start("../evil", ["mail"])).toThrow();
  expect(() => jobs.start("a@b", ["admin"])).toThrow();
  const j = jobs.start("a@b", ["calendar"]);
  expect(() => jobs.find(j.id, "other@b")).toThrow();
  expect(() => jobs.update("missing", { phase: "complete" })).toThrow();
});
it("expires interaction without retrying the rejected route forever", () => {
  let now = 1000;
  const jobs = new SignIns(store(), () => now);
  jobs.start("a@b", ["chat"]);
  now += 1801;
  expect(jobs.list()[0]).toMatchObject({ phase: "blocked" });
});
it("resumes the same isolated browser after an expired interaction", () => {
  const jobs = new SignIns(store());
  const first = jobs.start("a@b", ["mail"]);
  jobs.update(first.id, {
    slot: 4,
    phase: "blocked",
    url: "https://teams.example/api/desktop/4",
  });
  expect(jobs.start("a@b", ["mail"])).toMatchObject({
    slot: 4,
    phase: "queued",
  });
});
