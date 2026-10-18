import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import vm from "node:vm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chiama, METODI_TS, type NomeMetodoTs } from "@/lib/core/api";
import { archivio, chiudiArchivio } from "@/lib/core/archivio";
import type { Ev } from "@/lib/core/caselle";
import { adesso, fissaOra } from "@/lib/core/orologio";
import { creaMotore, creaStore, provaPromemoria } from "@/lib/core/promemoria";
import { autorizzazioneVapid, canalePush, chiaviVapid, cifra, disiscrivi, ISCRIZIONI_MAX, iscrivi, iscrizioni } from "@/lib/core/promemoria-push";
import { handle, type Deps } from "@/lib/gateway";
import { avviaPushFinto, browserFinto, chiaviVapidDiProva, decifra, soloRetiLocali } from "./push-finto";

// Le notifiche del browser: la cifratura contro l'esempio dell'RFC 8291, le chiavi VAPID, le iscrizioni, il canale contro un servizio di
// notifiche finto in locale, il motore, i metodi dell'API e il service worker. Mai un servizio di notifiche vero.

const NOMI_VAPID = ["VAPID_PUBLIC_KEY", "VAPID_PRIVATE_KEY", "VAPID_SUBJECT"];
const START = Date.UTC(2026, 9, 12, 13, 30, 0); // 15:30:00 a Roma
const ORA = START - 600_000;
const ENDPOINT = "https://push.example.test/send/abc123";

let dati: string;
let finto: Awaited<ReturnType<typeof avviaPushFinto>>;
let vietate: string[];
let log: string[];

const impostaChiavi = (extra: Record<string, string> = {}) => {
  const k = chiaviVapidDiProva();
  Object.assign(process.env, k, extra);
  return k;
};
const evento = (extra: Ev = {}): Ev => ({
  id: "prova|1|2026-10-12T15:30:00",
  account: "prova@example.test",
  alias: "Azienda di prova",
  title: "Riunione di prova",
  start: "2026-10-12T15:30:00",
  end: "2026-10-12T16:00:00",
  allDay: false,
  location: "Sala Rossa",
  online_url: null,
  ...extra,
});
const base64Url = (s: string) => Buffer.from(s, "base64url");

beforeEach(async () => {
  dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-push-"));
  process.env.OE_DATI = dati;
  delete process.env.GATEWAY_DB;
  for (const n of NOMI_VAPID) delete process.env[n];
  log = [];
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation((...a: unknown[]) => void log.push(a.map(String).join(" ")));
  finto = await avviaPushFinto();
  vietate = soloRetiLocali().vietate;
  fissaOra(ORA);
});

afterEach(async () => {
  await finto.chiudi();
  fissaOra(null);
  chiudiArchivio();
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", ...NOMI_VAPID]) delete process.env[n];
  vi.restoreAllMocks();
  expect(vietate).toEqual([]);
});

describe("la cifratura (RFC 8291)", () => {
  it("rifà l'esempio dell'RFC byte per byte", () => {
    const effimera = crypto.createECDH("prime256v1");
    effimera.setPrivateKey(base64Url("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"));
    const corpo = cifra(
      "When I grow up, I want to be a watermelon",
      base64Url("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"),
      base64Url("BTBZMqHH6r4Tts7J_aSIgg"),
      { salt: base64Url("DGv6ra1nlYgDCS1FRnbzlw"), effimera },
    );
    const atteso = base64Url(
      "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN",
    );
    expect(corpo.length).toBe(144);
    expect(corpo.toString("hex")).toBe(atteso.toString("hex"));
  });

  it("e il browser dell'RFC lo decifra con la sua chiave privata", () => {
    const ua = crypto.createECDH("prime256v1");
    ua.setPrivateKey(base64Url("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"));
    const corpo = base64Url(
      "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN",
    );
    expect(decifra(corpo, { ecdh: ua, auth: base64Url("BTBZMqHH6r4Tts7J_aSIgg") })).toBe("When I grow up, I want to be a watermelon");
  });

  it("con chiavi vere: si decifra, anche con accenti e emoji; due cifrature dello stesso testo sono diverse", () => {
    const b = browserFinto(ENDPOINT);
    const testo = "Tra 10 min, alle 15:30\nRiunione di è ù 😀";
    const a = cifra(testo, b.ecdh.getPublicKey(), b.auth);
    const c = cifra(testo, b.ecdh.getPublicKey(), b.auth);
    expect(decifra(a, b)).toBe(testo);
    expect(decifra(c, b)).toBe(testo);
    expect(a.equals(c)).toBe(false);
    // con il segreto di autenticazione sbagliato non si apre
    expect(() => decifra(a, { ecdh: b.ecdh, auth: crypto.randomBytes(16) })).toThrow();
  });
});

describe("le chiavi VAPID", () => {
  it("senza variabili, o con una sola, non ci sono", () => {
    expect(chiaviVapid()).toBeNull();
    process.env.VAPID_SUBJECT = "mailto:prova@example.test";
    expect(chiaviVapid()).toBeNull();
  });

  it("la coppia giusta con un contatto mailto: o https: vale; nessun'altra cosa", () => {
    impostaChiavi();
    expect(chiaviVapid()).not.toBeNull();
    process.env.VAPID_SUBJECT = "https://oe.example.test/contatto";
    expect(chiaviVapid()).not.toBeNull();
    for (const male of ["prova@example.test", "http://oe.example.test", "mailto:", "", "mailto:senza chiocciola"]) {
      process.env.VAPID_SUBJECT = male;
      expect(chiaviVapid(), male).toBeNull();
    }
  });

  it("la pubblica di un'altra coppia, una chiave corta o con caratteri che non sono base64url: non ci sono", () => {
    const k = impostaChiavi();
    process.env.VAPID_PUBLIC_KEY = chiaviVapidDiProva().VAPID_PUBLIC_KEY;
    expect(chiaviVapid()).toBeNull();
    process.env.VAPID_PUBLIC_KEY = k.VAPID_PUBLIC_KEY;
    expect(chiaviVapid()).not.toBeNull();
    for (const privata of [k.VAPID_PRIVATE_KEY.slice(0, 20), k.VAPID_PRIVATE_KEY + "AAAA", "non è base64url " + k.VAPID_PRIVATE_KEY.slice(0, 20)]) {
      process.env.VAPID_PRIVATE_KEY = privata;
      expect(chiaviVapid(), privata).toBeNull();
    }
    process.env.VAPID_PRIVATE_KEY = k.VAPID_PRIVATE_KEY;
    process.env.VAPID_PUBLIC_KEY = k.VAPID_PUBLIC_KEY.slice(2);
    expect(chiaviVapid()).toBeNull();
  });

  it("il comando del README (4.9) fa una coppia di chiavi che il server accetta", (ctx) => {
    const percorso = path.join(process.cwd(), "..", "..", "README.md");
    if (!fs.existsSync(percorso)) return ctx.skip("senza il README accanto (fuori dal repository)");
    const comando = /node -e "(const c=require\('crypto'\);[^"\r\n]+)"/.exec(fs.readFileSync(percorso, "utf-8"));
    expect(comando, "il comando per le chiavi VAPID non è più nel README").not.toBeNull();
    const uscita = execFileSync(process.execPath, ["-e", comando![1]], { encoding: "utf-8" });
    const righe = uscita.trim().split(/\r?\n/);
    expect(righe.map((r) => r.split("=")[0])).toEqual(["VAPID_PUBLIC_KEY", "VAPID_PRIVATE_KEY"]);
    for (const r of righe) process.env[r.slice(0, r.indexOf("="))] = r.slice(r.indexOf("=") + 1);
    process.env.VAPID_SUBJECT = "mailto:prova@example.test";
    expect(chiaviVapid()).not.toBeNull();
  });

  it("l'intestazione è un JWT ES256 per il servizio che riceve, valido 12 ore, firmato con la chiave privata", () => {
    const k = impostaChiavi();
    const chiavi = chiaviVapid()!;
    const intestazione = autorizzazioneVapid("https://fcm.example.test/fcm/send/xyz?a=1", chiavi, ORA);
    const m = /^vapid t=([\w-]+)\.([\w-]+)\.([\w-]+), k=([\w-]+)$/.exec(intestazione);
    expect(m).not.toBeNull();
    const [, h, c, f, pubblica] = m!;
    expect(pubblica).toBe(k.VAPID_PUBLIC_KEY);
    expect(JSON.parse(base64Url(h).toString())).toEqual({ typ: "JWT", alg: "ES256" });
    expect(JSON.parse(base64Url(c).toString())).toEqual({ aud: "https://fcm.example.test", exp: Math.floor(ORA / 1000) + 12 * 3600, sub: "mailto:prova@example.test" });
    const p = base64Url(k.VAPID_PUBLIC_KEY);
    const pubblicaChiave = crypto.createPublicKey({ key: { kty: "EC", crv: "P-256", x: p.subarray(1, 33).toString("base64url"), y: p.subarray(33).toString("base64url") }, format: "jwk" });
    expect(base64Url(f)).toHaveLength(64);
    expect(crypto.verify("sha256", Buffer.from(`${h}.${c}`), { key: pubblicaChiave, dsaEncoding: "ieee-p1363" }, base64Url(f))).toBe(true);
    expect(crypto.verify("sha256", Buffer.from(`${h}.${c}x`), { key: pubblicaChiave, dsaEncoding: "ieee-p1363" }, base64Url(f))).toBe(false);
  });
});

describe("le iscrizioni", () => {
  const valida = (extra: Record<string, unknown> = {}) => ({ ...browserFinto(ENDPOINT).iscrizione, ...extra });

  it("una iscrizione di un browser si salva, e la stessa si aggiorna", () => {
    const b = browserFinto(ENDPOINT);
    expect(iscrivi(b.iscrizione)).toEqual({ ok: true });
    const di = browserFinto(ENDPOINT);
    expect(iscrivi(di.iscrizione)).toEqual({ ok: true });
    expect(iscrizioni()).toEqual([{ endpoint: ENDPOINT, p256dh: di.iscrizione.keys.p256dh, auth: di.iscrizione.keys.auth }]);
    expect(iscrivi(browserFinto("https://updates.push.example.test/wpush/v2/xyz").iscrizione)).toEqual({ ok: true });
    expect(iscrizioni()).toHaveLength(2);
  });

  it("l'indirizzo deve essere https, un nome vero, porta 443, senza credenziali", () => {
    const no = (endpoint: unknown) => expect(iscrivi(valida({ endpoint })), String(endpoint)).toEqual({ ok: false, error: "L'iscrizione del browser non è valida." });
    no("http://push.example.test/x");
    no("https://127.0.0.1/x");
    no("https://10.0.0.5/x");
    no("https://[::1]/x");
    no("https://localhost/x");
    no("https://agent:8180/api/eventi");
    no("https://agent/api/eventi");
    no("https://servizio.internal/x");
    no("https://nas.local/x");
    no("https://push.example.test:8443/x");
    no("https://utente:segreto@push.example.test/x");
    no("https://push.example.test/x#frammento");
    no("ftp://push.example.test/x");
    no("push.example.test/x");
    no("https://push.example.test/" + "a".repeat(2100));
    no(null);
    no(5);
    no({ href: ENDPOINT });
    expect(iscrizioni()).toEqual([]);
  });

  it("le chiavi: 65 byte che sono un punto di P-256 e 16 byte di segreto", () => {
    const b = browserFinto(ENDPOINT).iscrizione;
    const no = (keys: unknown) => expect(iscrivi({ endpoint: ENDPOINT, keys }), JSON.stringify(keys)).toEqual({ ok: false, error: "L'iscrizione del browser non è valida." });
    no({ p256dh: b.keys.p256dh });
    no({ auth: b.keys.auth });
    no({ p256dh: b.keys.p256dh, auth: crypto.randomBytes(15).toString("base64url") });
    no({ p256dh: b.keys.p256dh, auth: crypto.randomBytes(17).toString("base64url") });
    no({ p256dh: b.keys.p256dh.slice(4), auth: b.keys.auth });
    no({ p256dh: Buffer.concat([Buffer.from([4]), crypto.randomBytes(64)]).toString("base64url"), auth: b.keys.auth }); // non è un punto della curva
    no({ p256dh: Buffer.concat([Buffer.from([3]), base64Url(b.keys.p256dh).subarray(1)]).toString("base64url"), auth: b.keys.auth });
    no({ p256dh: "non è base64url", auth: b.keys.auth });
    no({ p256dh: 5, auth: b.keys.auth });
    no("testo");
    no(null);
    expect(iscrivi({ endpoint: ENDPOINT })).toMatchObject({ ok: false });
    expect(iscrivi(null)).toMatchObject({ ok: false });
    expect(iscrizioni()).toEqual([]);
  });

  it(`al più ${ISCRIZIONI_MAX} browser: la ventunesima è rifiutata, quella già presente si aggiorna`, () => {
    for (let i = 0; i < ISCRIZIONI_MAX; i++) expect(iscrivi(browserFinto(`https://push.example.test/send/${i}`).iscrizione)).toEqual({ ok: true });
    expect(iscrivi(browserFinto("https://push.example.test/send/nuova").iscrizione)).toEqual({ ok: false, error: `Ci sono già ${ISCRIZIONI_MAX} browser iscritti.` });
    expect(iscrivi(browserFinto("https://push.example.test/send/3").iscrizione)).toEqual({ ok: true });
    expect(iscrizioni()).toHaveLength(ISCRIZIONI_MAX);
  });

  it("si toglie con il suo indirizzo; uno che non c'è o non vale non cambia niente", () => {
    iscrivi(browserFinto(ENDPOINT).iscrizione);
    expect(disiscrivi("https://push.example.test/send/altro")).toEqual({ ok: true });
    expect(disiscrivi(null)).toEqual({ ok: true });
    expect(disiscrivi("http://127.0.0.1/x")).toEqual({ ok: true });
    expect(iscrizioni()).toHaveLength(1);
    expect(disiscrivi(ENDPOINT)).toEqual({ ok: true });
    expect(iscrizioni()).toEqual([]);
  });

  it("la tabella push_iscrizioni sta in gateway.db, e le iscrizioni restano dopo che il database si riapre", () => {
    iscrivi(browserFinto(ENDPOINT).iscrizione);
    expect(fs.existsSync(path.join(dati, "gateway.db"))).toBe(true);
    const colonne = (archivio().prepare("PRAGMA table_info(push_iscrizioni)").all() as { name: string }[]).map((c) => c.name);
    expect(colonne).toEqual(["endpoint", "p256dh", "auth", "creata_il"]);
    chiudiArchivio();
    expect(iscrizioni()).toHaveLength(1);
  });
});

describe("il canale", () => {
  const canale = () => canalePush({ locali: true });
  const iscrittoQui = (percorso = "/push/uno") => {
    const b = browserFinto(`${finto.url}${percorso}`);
    expect(iscrivi(b.iscrizione, true)).toEqual({ ok: true });
    return b;
  };

  it("è configurato con le chiavi e almeno un browser iscritto; dettagli: le chiavi, la pubblica, quanti browser", () => {
    expect(canale().nome).toBe("push");
    expect(canale().configurato()).toBe(false);
    expect(canale().dettagli?.()).toEqual({ chiavi: false, chiave_pubblica: null, iscrizioni: 0 });
    const k = impostaChiavi();
    expect(canale().configurato()).toBe(false);
    expect(canale().dettagli?.()).toEqual({ chiavi: true, chiave_pubblica: k.VAPID_PUBLIC_KEY, iscrizioni: 0 });
    iscrittoQui();
    expect(canale().configurato()).toBe(true);
    expect(canale().dettagli?.()).toEqual({ chiavi: true, chiave_pubblica: k.VAPID_PUBLIC_KEY, iscrizioni: 1 });
    expect(JSON.stringify(canale().dettagli?.())).not.toContain(k.VAPID_PRIVATE_KEY);
  });

  it("senza chiavi, o senza browser iscritti, l'invio dice perché e non chiama nessuno", async () => {
    iscrittoQui();
    expect(await canale().invia("x")).toEqual({ ok: false, errore: "Notifiche del browser: chiavi VAPID non configurate sul server" });
    impostaChiavi();
    disiscrivi(`${finto.url}/push/uno`, true);
    expect(await canale().invia("x")).toEqual({ ok: false, errore: "Notifiche del browser: nessun browser iscritto" });
    expect(finto.richieste).toEqual([]);
  });

  it("POST all'indirizzo dell'iscrizione: intestazioni di RFC 8030, 8188 e 8292, e il corpo che il browser decifra", async () => {
    const k = impostaChiavi();
    const b = iscrittoQui("/push/uno");
    const testo = "Tra 10 min, alle 15:30\nRiunione di prova\nSala Rossa";
    expect(await canale().invia(testo)).toEqual({ ok: true });
    expect(finto.richieste).toHaveLength(1);
    const [r] = finto.richieste;
    expect(r.percorso).toBe("/push/uno");
    expect(r.intestazioni["content-encoding"]).toBe("aes128gcm");
    expect(r.intestazioni["content-type"]).toBe("application/octet-stream");
    expect(r.intestazioni.ttl).toBe("300");
    expect(r.intestazioni.urgency).toBe("high");
    const m = /^vapid t=([\w-]+)\.([\w-]+)\.([\w-]+), k=([\w-]+)$/.exec(String(r.intestazioni.authorization));
    expect(m?.[4]).toBe(k.VAPID_PUBLIC_KEY);
    expect(JSON.parse(base64Url(m![2]).toString())).toMatchObject({ aud: finto.url, sub: "mailto:prova@example.test" });
    expect(decifra(r.corpo, b)).toBe(testo);
    expect(r.corpo.length).toBeLessThanOrEqual(4096);
  });

  it("più browser: ognuno riceve il suo corpo, cifrato per lui; basta che uno lo riceva", async () => {
    impostaChiavi();
    const a = iscrittoQui("/push/a");
    const b = iscrittoQui("/push/b");
    finto.risposte.push({ stato: 201 }, { stato: 500 });
    expect(await canale().invia("Promemoria: prova")).toEqual({ ok: true });
    expect(finto.richieste.map((r) => r.percorso).sort()).toEqual(["/push/a", "/push/b"]);
    const per = (percorso: string) => finto.richieste.find((r) => r.percorso === percorso)!.corpo;
    expect(decifra(per("/push/a"), a)).toBe("Promemoria: prova");
    expect(decifra(per("/push/b"), b)).toBe("Promemoria: prova");
    expect(() => decifra(per("/push/a"), b)).toThrow();
    // il browser che ha dato 500 resta iscritto: è un guasto del servizio, non una iscrizione scaduta
    expect(iscrizioni()).toHaveLength(2);
  });

  it("404 e 410: l'iscrizione è scaduta e si toglie; le altre restano", async () => {
    impostaChiavi();
    iscrittoQui("/push/a");
    iscrittoQui("/push/b");
    iscrittoQui("/push/c");
    finto.risposte.push({ stato: 410 }, { stato: 404 }, { stato: 201 });
    expect(await canale().invia("x")).toEqual({ ok: true });
    expect(iscrizioni()).toHaveLength(1);
    // tutte scadute: non è un invio riuscito, e non resta nessuna iscrizione
    finto.risposte.push({ stato: 410 });
    const esito = await canale().invia("x");
    expect(esito).toEqual({ ok: false, errore: "Notifiche del browser: le iscrizioni erano scadute e si sono tolte, riattivale dalle impostazioni" });
    expect(iscrizioni()).toEqual([]);
    expect(canale().configurato()).toBe(false);
  });

  it("gli errori diventano frasi fisse: 429 con Retry-After, 401 e 403, 413, 5xx, altri", async () => {
    impostaChiavi();
    iscrittoQui();
    const chiedi = async (r: { stato: number; intestazioni?: Record<string, string> }) => {
      finto.risposte.push(r);
      const e = await canale().invia("x");
      return e.ok ? null : e;
    };
    expect(await chiedi({ stato: 429, intestazioni: { "Retry-After": "7" } })).toEqual({ ok: false, errore: "Notifiche del browser: il servizio limita gli invii (429)", attesaS: 7 });
    expect(await chiedi({ stato: 429 })).toMatchObject({ attesaS: 30 });
    expect((await chiedi({ stato: 401 }))?.errore).toContain("VAPID_PUBLIC_KEY");
    expect((await chiedi({ stato: 403 }))?.errore).toContain("(403)");
    expect((await chiedi({ stato: 413 }))?.errore).toBe("Notifiche del browser: messaggio troppo grande (413)");
    expect((await chiedi({ stato: 503 }))?.errore).toBe("Notifiche del browser: servizio non disponibile (503)");
    expect((await chiedi({ stato: 418 }))?.errore).toBe("Notifiche del browser: il servizio ha risposto 418");
    expect(iscrizioni()).toHaveLength(1);
  });

  it("un reindirizzamento non si segue: il server non va dove non è iscritto", async () => {
    impostaChiavi();
    iscrittoQui();
    finto.risposte.push({ stato: 302, intestazioni: { Location: `${finto.url}/altrove` } });
    expect(await canale().invia("x")).toEqual({ ok: false, errore: "Notifiche del browser: il servizio ha risposto 302" });
    expect(finto.richieste).toHaveLength(1);
  });

  it("nessuna risposta (qui in 150 ms): tempo scaduto; servizio che non c'è: non raggiungibile; niente indirizzi nel testo", async () => {
    impostaChiavi();
    iscrittoQui();
    finto.risposte.push({ stato: 201, ritardoMs: 1500 });
    expect(await canalePush({ locali: true, attesaMs: 150 }).invia("x")).toEqual({ ok: false, errore: "Notifiche del browser: il servizio non risponde (tempo scaduto)" });
    await finto.chiudi();
    const e = await canale().invia("x");
    expect(e).toEqual({ ok: false, errore: "Notifiche del browser: servizio non raggiungibile" });
    expect(JSON.stringify(e)).not.toContain("127.0.0.1");
  });

  it("un testo lungo si accorcia entro i byte che il servizio accetta, senza spezzare un carattere", async () => {
    impostaChiavi();
    const b = iscrittoQui();
    await canale().invia("é".repeat(3000) + "😀".repeat(200));
    const chiaro = decifra(finto.richieste[0].corpo, b);
    expect(Buffer.byteLength(chiaro)).toBeLessThanOrEqual(3800);
    expect(chiaro.endsWith("…")).toBe(true);
    expect(chiaro).not.toMatch(/[\ud800-\udbff]…$/);
    expect(finto.richieste[0].corpo.length).toBeLessThanOrEqual(4096);
  });

  it("il canale che non ammette indirizzi locali (quello vero) non chiama un'iscrizione locale", async () => {
    impostaChiavi();
    iscrittoQui();
    expect(await canalePush().invia("x")).toEqual({ ok: false, errore: "Notifiche del browser: nessun browser iscritto" });
    expect(finto.richieste).toEqual([]);
  });

  it("la prova dell'API passa dallo stesso canale: testo fisso, e un guasto dice perché", async () => {
    impostaChiavi();
    const b = iscrittoQui();
    expect(await provaPromemoria("push", [canale()])).toEqual({ ok: true });
    expect(decifra(finto.richieste[0].corpo, b)).toBe("Prova dei promemoria");
    finto.risposte.push({ stato: 503 });
    expect(await provaPromemoria("push", [canale()])).toEqual({ ok: false, error: "Notifiche del browser: servizio non disponibile (503)" });
  });
});

describe("con il motore", () => {
  it("il promemoria arriva cifrato al browser iscritto, una volta sola, con il titolo del messaggio come prima riga", async () => {
    impostaChiavi();
    const b = browserFinto(`${finto.url}/push/uno`);
    iscrivi(b.iscrizione, true);
    const m = creaMotore({
      adesso,
      leggiEventi: async () => ({ eventi: [evento()], problemi: 0 }),
      impostazioni: () => ({ minuti: [10], contenuto: "completo", suono: "Default", notifica_windows: true, comparsa: true }),
      canali: () => [canalePush({ locali: true })],
      store: creaStore(),
    });
    await m.giro();
    await m.giro();
    expect(finto.richieste).toHaveLength(1);
    expect(decifra(finto.richieste[0].corpo, b)).toBe("Tra 10 min, alle 15:30\nRiunione di prova\nSala Rossa");
    expect(archivio().prepare("SELECT canale, esito FROM promemoria_inviati").all()).toEqual([{ canale: "push", esito: "inviato" }]);
    // niente nei log: né il titolo, né il luogo, né l'indirizzo dell'iscrizione
    const tutto = log.join("\n");
    for (const s of ["Riunione di prova", "Sala Rossa", finto.url, "/push/uno"]) expect(tutto).not.toContain(s);
  });
});

describe("attraverso il gateway", () => {
  const deps = (utente: unknown): Deps => ({ user: async () => utente, loghi: [], metodi: { ha: (n) => METODI_TS.has(n), chiama: (n, a) => chiama(n as NomeMetodoTs, a) } });
  const post = (nome: string, corpo: unknown, headers: Record<string, string> = {}) =>
    new Request(`http://gateway:3000/api/${nome}`, { method: "POST", body: JSON.stringify(corpo), headers: { "content-type": "application/json", ...headers } });
  const api = async (nome: string, args: unknown[] = []) => {
    const r = await handle(post(nome, args), deps({ id: "u" }));
    return { stato: r.status, corpo: await r.json() };
  };

  it("sono metodi del gateway, con le regole di sempre: senza sessione 401, da un altro sito 403", async () => {
    for (const m of ["iscrivi_push", "disiscrivi_push"]) {
      expect(METODI_TS.has(m)).toBe(true);
      expect((await handle(post(m, [browserFinto(ENDPOINT).iscrizione]), deps(null))).status).toBe(401);
      expect((await handle(post(m, [browserFinto(ENDPOINT).iscrizione], { "sec-fetch-site": "cross-site" }), deps({ id: "u" }))).status).toBe(403);
      expect((await handle(new Request(`http://gateway:3000/api/${m}`, { method: "DELETE" }), deps({ id: "u" }))).status).toBe(405);
    }
    expect(iscrizioni()).toEqual([]);
  });

  it("l'iscrizione si salva e si toglie; una che non vale è una risposta di rifiuto; il canale compare in get_promemoria senza indirizzi né chiavi segrete", async () => {
    const k = impostaChiavi();
    const b = browserFinto(ENDPOINT);
    expect(await api("get_promemoria")).toMatchObject({
      corpo: { canali: [{ nome: "push", configurato: false, chiavi: true, chiave_pubblica: k.VAPID_PUBLIC_KEY, iscrizioni: 0 }] },
    });
    expect(await api("iscrivi_push", [b.iscrizione])).toEqual({ stato: 200, corpo: { ok: true } });
    expect(await api("iscrivi_push", [{ endpoint: "http://127.0.0.1/x", keys: b.iscrizione.keys }])).toEqual({ stato: 200, corpo: { ok: false, error: "L'iscrizione del browser non è valida." } });
    expect(await api("iscrivi_push", [])).toEqual({ stato: 200, corpo: { ok: false, error: "L'iscrizione del browser non è valida." } });
    const r = await api("get_promemoria");
    expect(r.corpo.canali.find((c: { nome: string }) => c.nome === "push")).toEqual({ nome: "push", configurato: true, chiavi: true, chiave_pubblica: k.VAPID_PUBLIC_KEY, iscrizioni: 1 });
    const testo = JSON.stringify(r);
    for (const segreto of [k.VAPID_PRIVATE_KEY, ENDPOINT, "/send/abc123", b.iscrizione.keys.auth, b.iscrizione.keys.p256dh]) expect(testo).not.toContain(segreto);
    expect(await api("disiscrivi_push", [ENDPOINT])).toEqual({ stato: 200, corpo: { ok: true } });
    expect(iscrizioni()).toEqual([]);
  });

  it("un browser che si iscrive accende il canale, l'ultimo che si toglie lo spegne: non c'è un interruttore del server", async () => {
    impostaChiavi();
    const pronto = async () => (await api("get_promemoria")).corpo.canali[0].configurato;
    expect(await pronto()).toBe(false);
    await api("iscrivi_push", [browserFinto(ENDPOINT).iscrizione]);
    expect(await pronto()).toBe(true);
    await api("disiscrivi_push", [ENDPOINT]);
    expect(await pronto()).toBe(false);
  });
});

describe("il service worker (public/sw.js)", () => {
  function carica() {
    const codice = fs.readFileSync(path.join(process.cwd(), "public", "sw.js"), "utf-8");
    const ascoltatori: Record<string, (e: unknown) => void> = {};
    const mostrate: [string, unknown][] = [];
    const attese: Promise<unknown>[] = [];
    const finestre: { focus: () => unknown }[] = [];
    const aperte: string[] = [];
    const self = {
      addEventListener: (nome: string, f: (e: unknown) => void) => void (ascoltatori[nome] = f),
      skipWaiting: vi.fn(),
      clients: { claim: vi.fn(), matchAll: async () => finestre, openWindow: (u: string) => void aperte.push(u) },
      registration: { showNotification: (t: string, o: unknown) => void mostrate.push([t, o]) },
    };
    vm.runInNewContext(codice, { self });
    const evento = (extra: object) => ({ waitUntil: (p: Promise<unknown>) => void attese.push(Promise.resolve(p)), ...extra });
    return { ascoltatori, mostrate, attese, finestre, aperte, self, evento };
  }

  it("un push diventa una notifica: la prima riga è il titolo, il resto il corpo", () => {
    const sw = carica();
    sw.ascoltatori.push(sw.evento({ data: { text: () => "Tra 10 min, alle 15:30\nRiunione di prova\nSala Rossa" } }));
    expect(sw.mostrate).toEqual([["Tra 10 min, alle 15:30", { body: "Riunione di prova\nSala Rossa", icon: "/icon.png" }]]);
  });

  it("senza testo c'è comunque una notifica (Chrome la chiede a ogni push)", () => {
    const sw = carica();
    sw.ascoltatori.push(sw.evento({}));
    expect(sw.mostrate).toEqual([["Promemoria", { body: "", icon: "/icon.png" }]]);
  });

  it("un clic sulla notifica la chiude e porta alla pagina: una finestra aperta o una nuova", async () => {
    const sw = carica();
    let chiusa = 0;
    sw.ascoltatori.notificationclick(sw.evento({ notification: { close: () => void chiusa++ } }));
    await Promise.all(sw.attese);
    expect(chiusa).toBe(1);
    expect(sw.aperte).toEqual(["/"]);
    let messaAFuoco = 0;
    sw.finestre.push({ focus: () => void messaAFuoco++ });
    sw.ascoltatori.notificationclick(sw.evento({ notification: { close: () => void chiusa++ } }));
    await Promise.all(sw.attese);
    expect(messaAFuoco).toBe(1);
    expect(sw.aperte).toEqual(["/"]);
  });

  it("si attiva subito (install e activate)", () => {
    const sw = carica();
    sw.ascoltatori.install(sw.evento({}));
    sw.ascoltatori.activate(sw.evento({}));
    expect(sw.self.skipWaiting).toHaveBeenCalled();
    expect(sw.self.clients.claim).toHaveBeenCalled();
  });
});
