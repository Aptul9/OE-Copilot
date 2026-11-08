import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { GestoreToken } from "../src/diretto.ts";
import type { SharedAccount } from "../src/shared-account.ts";
import { richiestaCondivisione, type ArchivioCondiviso } from "../src/shared-provisioning.ts";

// L'attivazione contro il GestoreToken vero, con Microsoft simulato: identità dal token rinnovato, prova di ogni risorsa con il rinnovo
// condiviso del nucleo. impostaShared qui scrive tokens.json come farà il metodo del GestoreToken (fuori da questo modulo).
const TID = "11111111-2222-3333-4444-555555555555";
const OID = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const jwt = (c: Record<string, unknown>) => "x." + Buffer.from(JSON.stringify(c)).toString("base64url") + ".x";

function scenario(rifiuta?: string) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "shared-prov-"));
  const file = path.join(dir, "tokens.json");
  fs.writeFileSync(file, JSON.stringify({ accounts: { "mario@example.com": { provider: "microsoft", refresh_token: "r0", tenant_id: TID } } }));
  const scopes: string[] = [];
  const fetchFn = (async (_url: string | URL | Request, init?: RequestInit) => {
    const body = new URLSearchParams(String(init?.body));
    const aud = String(body.get("scope")).split("/.default")[0]!.split(" ")[0]!;
    scopes.push(aud);
    if (rifiuta && aud.includes(rifiuta)) return Response.json({ error: "invalid_grant" }, { status: 400 });
    const audience = aud.startsWith("https://") ? aud : "https://outlook.office.com";
    return Response.json({
      access_token: jwt({ aud: audience, tid: TID, oid: OID, upn: "mario@example.com", exp: 2_000_000, scp: "Mail.ReadWrite Calendars.ReadWrite" }),
      refresh_token: `r${scopes.length}`,
      expires_in: 3600,
    });
  }) as typeof fetch;
  const gestore = new GestoreToken(file, { fetchFn, adesso: () => 1_000_000 });
  let tick = 0;
  const archivio = Object.assign(gestore, {
    impostaShared(email: string, shared: SharedAccount | undefined) {
      const dati = JSON.parse(fs.readFileSync(file, "utf8")) as { accounts: Record<string, Record<string, unknown>> };
      const voce = dati.accounts[email]!;
      if (shared) voce.shared = shared;
      else delete voce.shared;
      delete voce.resourceTokens;
      fs.writeFileSync(file, JSON.stringify(dati));
      const t = new Date(Date.now() + 5000 + ++tick * 1000);
      fs.utimesSync(file, t, t);
    },
  }) as unknown as ArchivioCondiviso;
  const salvato = () => (JSON.parse(fs.readFileSync(file, "utf8")) as { accounts: Record<string, { shared?: SharedAccount; refresh_token?: string }> }).accounts["mario@example.com"]!;
  return { archivio, scopes, salvato, fine: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

describe("attivazione con il GestoreToken", () => {
  it("salva identità dai claims e capacità, dopo aver ottenuto ogni risorsa", async () => {
    const s = scenario();
    try {
      const r = await richiestaCondivisione(s.archivio, "POST", { account: "mario@example.com", capabilities: ["mail", "calendar", "chat"] });
      expect(r).toEqual({ ok: true, account: "mario@example.com", shared: { capabilities: ["mail", "calendar", "chat"], origin: "mobile" } });
      expect(s.salvato().shared).toEqual({ tenantId: TID, objectId: OID, capabilities: ["mail", "calendar", "chat"], origin: "mobile" });
      expect(s.scopes).toEqual([
        expect.stringContaining("outlook"),
        "https://outlook.office.com",
        "https://chatsvcagg.teams.microsoft.com",
        "https://ic3.teams.office.com",
        "https://api.spaces.skype.com",
        "https://presence.teams.microsoft.com",
      ]);
    } finally {
      s.fine();
    }
  });

  it("Microsoft rifiuta la chat: la casella resta legacy, posta e calendario come prima", async () => {
    const s = scenario("ic3.teams");
    try {
      const r = await richiestaCondivisione(s.archivio, "POST", { account: "mario@example.com", capabilities: ["calendar", "chat"] });
      expect(r.ok).toBe(false);
      expect(String(r.errore)).toMatch(/chat.*ic3/);
      expect(s.salvato().shared).toBeUndefined();
    } finally {
      s.fine();
    }
  });
});
