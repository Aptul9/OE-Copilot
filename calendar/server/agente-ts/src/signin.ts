import crypto from "node:crypto";
import fs from "node:fs";

export type SignIn = {
  id: string;
  account: string;
  capabilities: ("mail" | "calendar" | "chat")[];
  phase: "queued" | "browser" | "complete" | "blocked";
  created: number;
  slot?: number;
  url?: string;
  error?: string;
  route?: string;
  // when it stopped (blocked): TeamsRelay gives back a slot it claimed for the sign-in only up to then
  ended?: number;
};

// a sign-in not finished in this time stops; its browser slot is given back
export const SIGNIN_TIMEOUT_S = 300;
export const CANCELLED = "Connection cancelled.";

export class SignIns {
  private readonly file: string;
  private readonly now: () => number;
  constructor(file: string, now = () => Math.floor(Date.now() / 1000)) {
    this.file = file;
    this.now = now;
  }
  private read(): SignIn[] {
    if (!fs.existsSync(this.file)) return [];
    return JSON.parse(fs.readFileSync(this.file, "utf8")) as SignIn[];
  }
  private save(jobs: SignIn[]) {
    fs.writeFileSync(this.file + ".tmp", JSON.stringify(jobs), { mode: 0o600 });
    fs.renameSync(this.file + ".tmp", this.file);
  }
  list(): SignIn[] {
    return this.read().map((j) =>
      j.phase !== "complete" &&
      j.phase !== "blocked" &&
      this.now() - j.created > SIGNIN_TIMEOUT_S
        ? {
            ...j,
            phase: "blocked",
            error: "Sign-in timed out. Resume when ready.",
            ended: j.created + SIGNIN_TIMEOUT_S,
          }
        : j,
    );
  }
  start(address: string, capabilities: unknown): SignIn {
    const account = address.trim().toLowerCase();
    if (!/^[^\s@/?#]{1,128}@[^\s@/?#]{1,253}$/.test(account))
      throw new Error("Valid account address required");
    if (
      !Array.isArray(capabilities) ||
      !capabilities.length ||
      capabilities.some((c) => !["mail", "calendar", "chat"].includes(c))
    )
      throw new Error("Choose mail, calendar or Teams");
    const jobs = this.list();
    const previous = jobs.find((j) => j.account === account);
    if (previous && ["queued", "browser"].includes(previous.phase)) {
      if (
        JSON.stringify([...new Set(capabilities)].sort()) !==
        JSON.stringify([...previous.capabilities].sort())
      )
        throw new Error(
          "A connection is already in progress. Finish it before changing services.",
        );
      return previous;
    }
    const job: SignIn = {
      id: crypto.randomUUID(),
      account,
      capabilities: [...new Set(capabilities)],
      phase: "queued",
      created: this.now(),
      ...(previous?.slot ? { slot: previous.slot, url: previous.url } : {}),
    };
    this.save([...jobs.filter((j) => j.account !== account).slice(-63), job]);
    return job;
  }
  find(id: unknown, account: unknown): SignIn {
    const j = this.list().find((j) => j.id === id && j.account === account);
    if (!j || ["complete", "blocked"].includes(j.phase))
      throw new Error("Sign-in request expired or mismatched");
    return j;
  }
  update(
    id: string,
    patch: Partial<Omit<SignIn, "id" | "account" | "capabilities" | "created">>,
  ): SignIn {
    const jobs = this.read();
    const j = jobs.find((j) => j.id === id);
    if (!j) throw new Error("Sign-in request not found");
    if (patch.phase === "blocked" && j.phase !== "blocked" && !patch.ended)
      patch = { ...patch, ended: this.now() };
    Object.assign(j, patch);
    this.save(jobs);
    return j;
  }
  // The owner stops a sign-in, running or already stopped: it no longer shows, and its slot is given back. A finished one
  // stays as it is.
  cancel(id: unknown, account: unknown): SignIn {
    const j = this.list().find((j) => j.id === id && j.account === account);
    if (!j) throw new Error("Sign-in request not found");
    if (j.phase === "complete") throw new Error("Sign-in already complete");
    return this.update(j.id, {
      phase: "blocked",
      error: CANCELLED,
      ended: j.phase === "blocked" ? (j.ended ?? this.now()) : this.now(),
    });
  }
}
