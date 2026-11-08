import { describe, expect, it } from "vitest";
import { attivaCondivisione, capacitaValide, disattivaCondivisione, origineDi, richiestaCondivisione, type ArchivioCondiviso } from "../src/shared-provisioning.ts";
import { OUTLOOK_WEB_CLIENT_ID } from "../src/sessione-web.ts";
import type { SharedAccount } from "../src/shared-account.ts";

const TID = "11111111-2222-3333-4444-555555555555";
const OID = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const jwt = (claims: Record<string, unknown>) => "x." + Buffer.from(JSON.stringify(claims)).toString("base64url") + ".x";

type Voce = NonNullable<ReturnType<ArchivioCondiviso["voceAccount"]>>;

function archivio(voci: Record<string, Voce>, o: { token?: string; rifiuta?: string } = {}) {
  const scritture: (SharedAccount | undefined)[] = [];
  const richieste: string[] = [];
  const forzati: boolean[] = [];
  const a: ArchivioCondiviso = {
    voceAccount: (e) => voci[e.toLowerCase()],
    elencoAccount: () => voci,
    ottieniAccessToken: async (_e, forza) => {
      forzati.push(!!forza);
      return o.token ?? jwt({ tid: TID, oid: OID, upn: "Mario@Example.com" });
    },
    ottieniRisorsa: async (_e, r, force) => {
      expect(force).toBe(true);
      richieste.push(r);
      if (r === o.rifiuta) throw new Error("shared renewal refused");
      return "t";
    },
    impostaShared: (e, s) => {
      scritture.push(s);
      voci[e]!.shared = s;
    },
  };
  return { a, scritture, richieste, forzati };
}

describe("attivazione dell'account condiviso", () => {
  it("prende tenant e object id dal token rinnovato, non dalla richiesta, e prova ogni risorsa della chat", async () => {
    const voci: Record<string, Voce> = { "mario@example.com": { provider: "microsoft", refresh_token: "r" } };
    const s = archivio(voci);
    const r = await richiestaCondivisione(s.a, "POST", { account: " Mario@Example.com ", capabilities: ["chat", "mail", "chat"], tenantId: "evil", objectId: "evil" });
    expect(r).toEqual({ ok: true, account: "mario@example.com", shared: { capabilities: ["mail", "chat"], origin: "mobile" } });
    expect(s.forzati).toEqual([true]);
    expect(s.scritture).toEqual([{ tenantId: TID, objectId: OID, capabilities: ["mail", "chat"], origin: "mobile" }]);
    expect(s.richieste).toEqual(["outlook", "csa", "ic3", "spaces", "presence"]);
  });

  it("una capacità rifiutata da Microsoft rimette la casella com'era", async () => {
    const voci: Record<string, Voce> = { "mario@example.com": { provider: "microsoft", refresh_token: "r" } };
    const s = archivio(voci, { rifiuta: "ic3" });
    await expect(attivaCondivisione(s.a, "mario@example.com", ["calendar", "chat"])).rejects.toThrow(/chat.*ic3/);
    expect(s.scritture.at(-1)).toBeUndefined();
    expect(voci["mario@example.com"]!.shared).toBeUndefined();
  });

  it("cambiare le capacità tiene identità e origine già salvate, senza un nuovo token", async () => {
    const prima: SharedAccount = { tenantId: TID, objectId: OID, capabilities: ["mail", "calendar", "chat"], origin: "outlook-spa" };
    const voci: Record<string, Voce> = { "mario@example.com": { provider: "microsoft", refresh_token: "r", shared: prima } };
    const s = archivio(voci, { rifiuta: "csa" });
    await attivaCondivisione(s.a, "mario@example.com", ["calendar"]);
    expect(s.forzati).toEqual([]);
    expect(voci["mario@example.com"]!.shared).toEqual({ ...prima, capabilities: ["calendar"] });
    // la chat non c'è più: nessuna prova sulle sue risorse
    expect(s.richieste).toEqual(["outlook"]);
  });

  it.each([
    ["di un'altra casella", jwt({ tid: TID, oid: OID, upn: "altro@example.com" })],
    ["senza tenant", jwt({ oid: OID, upn: "mario@example.com" })],
    ["con object id non GUID", jwt({ tid: TID, oid: "x", upn: "mario@example.com" })],
    ["che non è un JWT", "opaque"],
  ])("rifiuta un token %s senza scrivere nulla", async (_n, token) => {
    const voci: Record<string, Voce> = { "mario@example.com": { provider: "microsoft", refresh_token: "r" } };
    const s = archivio(voci, { token });
    await expect(attivaCondivisione(s.a, "mario@example.com", ["mail"])).rejects.toThrow();
    expect(s.scritture).toEqual([]);
  });

  it("rifiuta Google, una casella senza refresh token, una con l'accesso scaduto, una sconosciuta", async () => {
    const voci: Record<string, Voce> = {
      "g@example.com": { provider: "google", refresh_token: "r" },
      "n@example.com": { provider: "microsoft" },
      "e@example.com": { provider: "microsoft", refresh_token: "r", errore: "Accesso scaduto" },
    };
    const s = archivio(voci);
    for (const account of ["g@example.com", "n@example.com", "e@example.com", "x@example.com"]) {
      expect((await richiestaCondivisione(s.a, "POST", { account, capabilities: ["mail"] })).ok).toBe(false);
    }
    expect(s.scritture).toEqual([]);
  });

  it("l'origine viene dal client dell'accesso", () => {
    expect(origineDi(OUTLOOK_WEB_CLIENT_ID)).toBe("outlook-spa");
    expect(origineDi("5e3ce6c0-2b1f-4285-8d4b-75ee78787346")).toBe("teams-spa");
    expect(origineDi(undefined)).toBe("mobile");
  });

  it("accetta solo capacità note, almeno una", () => {
    for (const v of [[], ["admin"], "mail", null, [1]]) expect(() => capacitaValide(v)).toThrow();
  });
});

describe("stato e spegnimento", () => {
  it("GET elenca le caselle Microsoft con capacità e origine, senza tenant e object id", async () => {
    const voci: Record<string, Voce> = {
      "a@example.com": { provider: "microsoft", shared: { tenantId: TID, objectId: OID, capabilities: ["chat"], origin: "mobile" } },
      "b@example.com": { provider: "microsoft" },
      "g@example.com": { provider: "google" },
    };
    const r = await richiestaCondivisione(archivio(voci).a, "GET", null);
    expect(r).toEqual({ ok: true, accounts: [{ account: "a@example.com", shared: { capabilities: ["chat"], origin: "mobile" } }, { account: "b@example.com", shared: null }] });
    expect(JSON.stringify(r)).not.toContain(OID);
  });

  it("enabled false toglie i metadati e lascia la casella", async () => {
    const voci: Record<string, Voce> = { "a@example.com": { provider: "microsoft", refresh_token: "r", shared: { tenantId: TID, objectId: OID, capabilities: ["chat"], origin: "mobile" } } };
    const s = archivio(voci);
    expect(await richiestaCondivisione(s.a, "POST", { account: "a@example.com", enabled: false })).toEqual({ ok: true, account: "a@example.com", shared: null });
    expect(s.scritture).toEqual([undefined]);
    expect(voci["a@example.com"]).toEqual({ provider: "microsoft", refresh_token: "r", shared: undefined });
    expect(disattivaCondivisione(s.a, "a@example.com").shared).toBeNull();
    expect(s.scritture).toHaveLength(1);
  });

  it("senza gestore token risponde con l'errore", async () => {
    expect(await richiestaCondivisione(undefined, "GET", null)).toEqual({ ok: false, errore: "gestore token non configurato" });
  });
});
