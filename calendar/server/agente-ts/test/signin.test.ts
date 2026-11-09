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
it("times out after five minutes, ended at that moment", () => {
  let now = 1000;
  const jobs = new SignIns(store(), () => now);
  jobs.start("a@b", ["mail", "chat"]);
  now += 300;
  expect(jobs.list()[0]?.phase).toBe("queued");
  now += 1;
  expect(jobs.list()[0]).toMatchObject({ phase: "blocked", error: expect.stringContaining("timed out"), ended: 1300 });
});
it("records when a sign-in stopped, once", () => {
  let now = 1000;
  const jobs = new SignIns(store(), () => now);
  const j = jobs.start("a@b", ["mail"]);
  now = 1050;
  jobs.update(j.id, { phase: "blocked", error: "refused" });
  now = 1100;
  jobs.update(j.id, { phase: "blocked", error: "refused again" });
  expect(jobs.list()[0]).toMatchObject({ phase: "blocked", ended: 1050 });
});
it("cancels a running or stopped sign-in, never a finished one", () => {
  let now = 1000;
  const jobs = new SignIns(store(), () => now);
  const running = jobs.start("a@b", ["mail", "chat"]);
  now = 1020;
  expect(jobs.cancel(running.id, "a@b")).toMatchObject({ phase: "blocked", error: "Connection cancelled.", ended: 1020 });
  const late = jobs.start("c@d", ["mail"]);
  now = 1400;
  // timed out at 1320: cancelling only hides it, the end stays
  expect(jobs.cancel(late.id, "c@d")).toMatchObject({ error: "Connection cancelled.", ended: 1320 });
  const done = jobs.start("e@f", ["mail"]);
  jobs.update(done.id, { phase: "complete" });
  expect(() => jobs.cancel(done.id, "e@f")).toThrow();
  expect(() => jobs.cancel(running.id, "other@b")).toThrow();
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
