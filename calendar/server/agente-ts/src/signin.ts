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
};

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
      this.now() - j.created > 1800
        ? {
            ...j,
            phase: "blocked",
            error: "Sign-in timed out. Resume when ready.",
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
    Object.assign(j, patch);
    this.save(jobs);
    return j;
  }
}
