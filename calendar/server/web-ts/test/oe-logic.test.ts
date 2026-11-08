import { describe, expect, it } from "vitest";
import { ApiError, creaApi } from "@/lib/oe/api";
import {
  azioniEvento,
  cambiaSpenti,
  chiaveGiorno,
  coloriDinamici,
  corrisponde,
  dentroLaCache,
  dovePagina,
  aScorrimento,
  giorniDiArrivo,
  etichettaSync,
  eventiPerGriglia,
  eventoPerGriglia,
  infoApp,
  filtraAziende,
  inAlto,
  spezzaAl,
  dataEvento,
  limitaLarghezza,
  LARGHEZZA_PANNELLO,
  secondariEffettivi,
  soloDelDispositivo,
  soloDelServer,
  unisciConfig,
  separaSecondari,
  fmtRange,
  orariInCima,
  giorniTra,
  iniziali,
  urlLogo,
  daDatetimeLocal,
  inOraDelDispositivo,
  isoLocale,
  periodoLargo,
  raccogliFestivi,
  raggruppaAziende,
  salute,
  schedeTimesheet,
  sigleScheda,
  spostaGiorni,
  testoDatiVecchi,
  tintaOpaca,
  TINTE_DINAMICHE,
  vicinoAlBordo,
  vistaPerSchermo,
} from "@/lib/oe/logic";
import type { Calendario, Evento } from "@/lib/oe/types";

const ev = (extra: Partial<Evento> = {}): Evento => ({
  id: "android::a@x.it|1|2026-10-12T09:30:00",
  title: "Riunione",
  start: "2026-10-12T09:30:00",
  end: "2026-10-12T10:00:00",
  allDay: false,
  color: "#2563eb",
  account: "a@x.it",
  alias: "ROAR",
  remoto: true,
  online_url: "https://teams.microsoft.com/l/meetup-join/1",
  ...extra,
});

describe("iniziali", () => {
  it.each([
    ["Alfa", "AL"],
    ["Beta Servizi", "BS"],
    ["Alfa · m.rossi", "AL"],
    ["antonio.maritato@itv.com", "AM"],
    ["", ""],
  ])("%s -> %s", (nome, atteso) => expect(iniziali(nome)).toBe(atteso));
});

describe("colori", () => {
  it("il riempimento è un mix senza trasparenza con la superficie del tema", () => {
    expect(tintaOpaca("#2563eb", 14)).toBe("color-mix(in srgb, #2563eb 14%, var(--card))");
    expect(tintaOpaca("#2563eb", 14)).not.toMatch(/rgba|transparent/);
  });
});

describe("date", () => {
  it("giorniTra e spostaGiorni passano il cambio dell'ora legale", () => {
    const a = new Date(2026, 9, 24);
    expect(giorniTra(a, new Date(2026, 9, 27))).toBe(3);
    expect(giorniTra(a, spostaGiorni(a, 3))).toBe(3);
    expect(chiaveGiorno(spostaGiorni(new Date(2026, 9, 31), 1))).toBe("2026-11-01");
  });
  it("isoLocale ha lo scarto orario", () => {
    expect(isoLocale(new Date(2026, 9, 11, 0, 0, 0))).toMatch(/^2026-10-11T00:00:00[+-]\d\d:\d\d$/);
  });
  it("gli orari di Roma diventano istanti, i giorni interi e gli orari con lo scarto restano", () => {
    const ev = (start: string, end: string, allDay = false) => ({ id: "1", title: "t", start, end, allDay, color: "", account: "A" }) as Evento;
    expect(inOraDelDispositivo(ev("2026-10-22T10:00:00", "2026-10-22T11:00:00"))).toMatchObject({ start: "2026-10-22T08:00:00.000Z", end: "2026-10-22T09:00:00.000Z" });
    expect(inOraDelDispositivo(ev("2026-12-01T10:00:00", "2026-12-01T10:30:00")).start).toBe("2026-12-01T09:00:00.000Z");
    expect(inOraDelDispositivo(ev("2026-10-22", "2026-10-23", true))).toMatchObject({ start: "2026-10-22", end: "2026-10-23" });
    expect(inOraDelDispositivo(ev("2026-10-22T10:00:00+03:00", "illeggibile"))).toMatchObject({ start: "2026-10-22T10:00:00+03:00", end: "illeggibile" });
  });
  it("daDatetimeLocal: l'ora del dispositivo come ora di Roma", () => {
    const ms = new Date("2026-10-22T11:00").getTime();
    expect(new Date(inOraDelDispositivo({ start: daDatetimeLocal("2026-10-22T11:00"), end: "", allDay: false } as Evento).start).getTime()).toBe(ms);
    expect(daDatetimeLocal("")).toBe("");
  });
  it("fmtRange", () => {
    const s = new Date(2026, 9, 12, 9, 30);
    const e = new Date(2026, 9, 12, 10, 0);
    expect(fmtRange(s, e, false)).toMatch(/^Lunedì 12 ottobre · 09:30–10:00$/);
    expect(fmtRange(s, e, true)).toBe("Lunedì 12 ottobre · tutto il giorno");
  });
  it("un evento di più giorni ha anche il giorno della fine; una fine a mezzanotte resta del giorno prima", () => {
    // la finestra di un change di ServiceNow: 19 ottobre 14:45 - 30 novembre 09:20
    const s = new Date(2026, 9, 19, 14, 45);
    const e = new Date(2026, 10, 30, 9, 20);
    expect(fmtRange(s, e, false)).toBe("Lunedì 19 ottobre, 14:45 – lunedì 30 novembre, 09:20");
    expect(orariInCima(s, e)).toBe("19 ott 14:45 – 30 nov 09:20");
    expect(fmtRange(new Date(2026, 9, 26, 18, 0), new Date(2026, 9, 27, 0, 0), false)).toBe("Lunedì 26 ottobre · 18:00–00:00");
    expect(orariInCima(new Date(2026, 9, 26, 9, 0), new Date(2026, 9, 26, 22, 0))).toBe("09:00–22:00");
    expect(fmtRange(new Date(2026, 9, 12), new Date(2026, 9, 15), true)).toBe("Lunedì 12 ottobre – mercoledì 14 ottobre · tutto il giorno");
    expect(fmtRange(new Date(2026, 9, 12), new Date(2026, 9, 13), true)).toBe("Lunedì 12 ottobre · tutto il giorno");
  });
});

describe("festività e giorni interi", () => {
  it("la fine esclusiva non tinge il giorno dopo", () => {
    const r = raccogliFestivi([ev({ allDay: true, title: "Ognissanti", account: "Festività", start: "2026-11-01T00:00:00", end: "2026-11-02T00:00:00" })]);
    expect(Object.keys(r.festivi)).toEqual(["2026-11-01"]);
  });
  it("un evento intero di una casella non è una festività", () => {
    const r = raccogliFestivi([ev({ allDay: true, title: "Ferie", start: "2026-10-12T00:00:00", end: "2026-10-14T00:00:00" })]);
    expect(r.festivi).toEqual({});
  });
});

describe("riunioni lunghe in cima (inAlto)", () => {
  // orari del dispositivo come li riceve la griglia (inOraDelDispositivo): istanti
  const lunga = (da: Date, a: Date, extra: Partial<Evento> = {}) => eventoPerGriglia(ev({ start: da.toISOString(), end: a.toISOString(), ...extra }), new Set());
  it("sotto le 4 ore resta nella griglia", () => {
    const e = lunga(new Date(2026, 9, 26, 9, 0), new Date(2026, 9, 26, 12, 59));
    expect(inAlto(e)).toBe(e);
  });
  it("da 4 ore in su va nella riga dei giorni interi, con gli orari veri nell'evento", () => {
    const e = inAlto(lunga(new Date(2026, 9, 26, 9, 0), new Date(2026, 9, 26, 22, 30)));
    expect(e).toMatchObject({ allDay: true, start: "2026-10-26", end: "2026-10-27" });
    expect(new Date(e.extendedProps.start).getHours()).toBe(9);
  });
  it("su più giorni tocca tutti i giorni, e una fine a mezzanotte non tocca il giorno dopo", () => {
    expect(inAlto(lunga(new Date(2026, 9, 25, 22, 0), new Date(2026, 9, 27, 6, 0)))).toMatchObject({ start: "2026-10-25", end: "2026-10-28" });
    expect(inAlto(lunga(new Date(2026, 9, 26, 18, 0), new Date(2026, 9, 27, 0, 0)))).toMatchObject({ start: "2026-10-26", end: "2026-10-27" });
  });
  it("una barra in cima che comincia fra i giorni nascosti si divide al primo giorno che si vede", () => {
    const e = inAlto(lunga(new Date(2026, 9, 23, 14, 45), new Date(2026, 9, 26, 9, 20)));
    const [prima, dopo] = spezzaAl(e, "2026-10-26");
    expect([prima.start, prima.end, dopo.start, dopo.end]).toEqual(["2026-10-23", "2026-10-26", "2026-10-26", "2026-10-27"]);
    expect(dopo.id).not.toBe(prima.id);
    expect(dopo.extendedProps).toBe(e.extendedProps);
    // comincia dal giorno che si vede, finisce prima, o non è in cima: resta intera
    expect(spezzaAl(e, "2026-10-23")).toEqual([e]);
    expect(spezzaAl(e, "2026-10-27")).toEqual([e]);
    const sotto = lunga(new Date(2026, 9, 26, 9, 0), new Date(2026, 9, 26, 10, 0));
    expect(spezzaAl(sotto, "2026-10-26")).toEqual([sotto]);
  });
  it("dataEvento legge un giorno intero come mezzanotte del dispositivo", () => {
    expect(dataEvento("2026-10-26", true).getDate()).toBe(26);
    expect(dataEvento("2026-10-26T00:00:00", true).getHours()).toBe(0);
    expect(dataEvento("2026-10-26T07:00:00.000Z", false).getTime()).toBe(Date.UTC(2026, 9, 26, 7));
  });
  it("un giorno intero e una fine illeggibile restano come sono", () => {
    const intero = eventoPerGriglia(ev({ allDay: true, start: "2026-10-26", end: "2026-10-27" }), new Set());
    expect(inAlto(intero)).toBe(intero);
    const rotto = eventoPerGriglia(ev({ end: "illeggibile" }), new Set());
    expect(inAlto(rotto)).toBe(rotto);
  });
});

describe("legenda", () => {
  const cal = (extra: Partial<Calendario>): Calendario => ({ id: "x", account: "a@x.it", label: "a@x.it · Calendario", color: "#111", enabled: true, stato: "ok", ...extra });
  it("raggruppa per azienda mostrata e conta quelli accesi", () => {
    const g = raggruppaAziende([
      cal({ id: "1", alias: "ROAR" }),
      cal({ id: "2", alias: "ROAR", account: "b@x.it", enabled: false }),
      cal({ id: "3", account: "c@x.it" }),
    ]);
    expect(g.map((x) => [x.azienda, x.tot, x.on, x.caselle.length])).toEqual([
      ["ROAR", 2, 1, 2],
      ["c@x.it", 1, 1, 1],
    ]);
  });
  it("basta un calendario in errore: rotto; un avviso resta avviso", () => {
    const g = raggruppaAziende([cal({ alias: "A", stato: "avviso", stato_msg: "in ritardo" }), cal({ alias: "B", stato: "errore", stato_msg: "non risponde" })]);
    expect([g[0].avviso, g[0].rotto, g[1].avviso, g[1].rotto]).toEqual([true, false, false, true]);
  });
  it("il pallino segue l'ultima lettura, poi lo stato dei calendari", () => {
    const g = raggruppaAziende([cal({ alias: "A", stato: "errore", stato_msg: "calendario rotto" })])[0];
    expect(salute({ A: { stato: "ok", msg: "Letto alle 07:48" } }, g)).toEqual({ classe: "good", titolo: "Letto alle 07:48" });
    expect(salute({}, g)).toEqual({ classe: "bad", titolo: "calendario rotto" });
    expect(salute({}, raggruppaAziende([cal({ alias: "A" })])[0])).toEqual({ classe: "warn", titolo: "Non ancora letto" });
    expect(salute({ A: { stato: "ok", msg: "" } }, g, true).classe).toBe("attesa");
  });
});

describe("eventi per la griglia", () => {
  it("riempimento opaco; secondari più tenui con bordo tratteggiato dalla classe", () => {
    const [n] = eventiPerGriglia([ev()], { secondari: new Set(), ricerca: "" });
    expect(n.backgroundColor).toBe("color-mix(in srgb, #2563eb 14%, var(--card))");
    expect(n.borderColor).toBe("#2563eb");
    expect(n.classNames).toEqual([]);
    const [s] = eventiPerGriglia([ev()], { secondari: new Set(["ROAR"]), ricerca: "" });
    expect(s.backgroundColor).toContain("7%");
    expect(s.borderColor).toContain("55%");
    expect(s.classNames).toEqual(["secondario"]);
  });
  it("tiene fuori i nascosti e le festività, e filtra con la ricerca", () => {
    const eventi = [ev({ id: "1" }), ev({ id: "2", nascosto: true }), ev({ id: "3", account: "Festività" }), ev({ id: "4", title: "Daily Bandpey", alias: "Bandpey" })];
    expect(eventiPerGriglia(eventi, { secondari: new Set(), ricerca: "" }).map((e) => e.id)).toEqual(["1", "4"]);
    expect(eventiPerGriglia(eventi, { secondari: new Set(), ricerca: "  bandpey " }).map((e) => e.id)).toEqual(["4"]);
    expect(eventiPerGriglia(eventi, { secondari: new Set(), ricerca: "zzzz" })).toEqual([]);
  });
  it("senzaTuttoIlGiorno toglie i giorni interi (anche Occupato) e gli eventi che durano un giorno o più, non le riunioni con l'orario", () => {
    const eventi = [
      ev({ id: "riunione" }),
      ev({ id: "intero", allDay: true, start: "2026-10-12", end: "2026-10-13" }),
      ev({ id: "intero-occupato", allDay: true, occupato: true, start: "2026-10-12", end: "2026-10-13" }),
      ev({ id: "blocco-di-un-mese", start: "2026-10-19T14:45:00", end: "2026-11-30T09:20:00" }),
      ev({ id: "un-giorno-con-orario", start: "2026-10-12T00:00:00", end: "2026-10-13T00:00:00" }),
      ev({ id: "quasi-un-giorno", start: "2026-10-12T00:00:00", end: "2026-10-12T23:59:00" }),
      ev({ id: "riunione-lunga", start: "2026-10-29T17:00:00", end: "2026-10-29T21:30:00", occupato: true }),
    ];
    const ids = (senzaTuttoIlGiorno?: boolean, ricerca = "") =>
      eventiPerGriglia(eventi, { secondari: new Set(), ricerca, senzaTuttoIlGiorno }).map((e) => e.id);
    const tutti = ["riunione", "intero", "intero-occupato", "blocco-di-un-mese", "un-giorno-con-orario", "quasi-un-giorno", "riunione-lunga"];
    expect(ids()).toEqual(tutti);
    expect(ids(false)).toEqual(tutti);
    expect(ids(true)).toEqual(["riunione", "quasi-un-giorno", "riunione-lunga"]);
    expect(ids(true, "zzzz")).toEqual([]);
  });
  it("la ricerca guarda titolo, azienda, indirizzo, calendario, luogo e organizzatore", () => {
    const e = ev({ location: "Sala Verdi", organizer: "Mario Rossi", calendar: "a@x.it · Team" });
    for (const q of ["riunione", "roar", "a@x.it", "team", "verdi", "rossi"]) expect(corrisponde(e, q)).toBe(true);
    expect(corrisponde(e, "altro")).toBe(false);
  });
});

describe("scheda dell'evento", () => {
  it("riunione Teams di una casella: Partecipa e Apri in Teams", () => {
    const a = azioniEvento(ev(), new Set());
    expect(a.partecipa).toBe("https://teams.microsoft.com/l/meetup-join/1");
    expect(a.apriInTeams).toEqual({ account: "a@x.it", url: "https://teams.microsoft.com/l/meetup-join/1" });
    expect(a.elimina).toBeNull();
  });
  it("niente Apri in Teams per le caselle senza Teams, le copie inoltrate e le riunioni senza link", () => {
    expect(azioniEvento(ev(), new Set(["ROAR"])).apriInTeams).toBeNull();
    expect(azioniEvento(ev({ inoltrata_da: "x@y.it" }), new Set()).apriInTeams).toBeNull();
    const senza = azioniEvento(ev({ online_url: null }), new Set());
    expect(senza.partecipa).toBeNull();
    expect(senza.apriInTeams).toBeNull();
  });
  it("una riunione Teams di un calendario Google compare, una non Teams no", () => {
    expect(azioniEvento(ev({ is_google: true }), new Set()).apriInTeams).not.toBeNull();
    expect(azioniEvento(ev({ is_google: true, online_url: "https://meet.google.com/abc" }), new Set()).apriInTeams).toBeNull();
  });
  it("un evento personale si elimina", () => {
    expect(azioniEvento(ev({ is_local: true, uid: "u1", remoto: false, online_url: null }), new Set()).elimina).toBe("u1");
  });
});

describe("sincronizzazione", () => {
  it("l'etichetta", () => {
    const t = 1_000_000;
    expect(etichettaSync(t, t, 1)).toBe("Sincronizzo…");
    expect(etichettaSync(null, t, 0)).toBe("—");
    expect(etichettaSync(t, t + 4000, 0)).toBe("Sincronizzato ora");
    expect(etichettaSync(t, t + 30_000, 0)).toBe("Sincronizzato 30 sec fa");
    expect(etichettaSync(t, t + 5 * 60_000, 0)).toBe("Sincronizzato 5 min fa");
    expect(etichettaSync(t, t + 3 * 3600_000, 0)).toBe("Sincronizzato 3 h fa");
  });
  it("i dati vecchi", () => {
    expect(testoDatiVecchi("2026-10-11T07:23:02", "Il server non risponde")).toBe("Il server non risponde. Dati aggiornati alle 07:23.");
    expect(testoDatiVecchi(null, null)).toBe("Il server dei calendari non risponde.");
  });
});

describe("dove si apre la pagina", () => {
  const WEBVIEW2 = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0";
  const WEBVIEW_ANDROID = "Mozilla/5.0 (Linux; Android 13; Pixel 7 Build/TQ3A.230901.001; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/141.0.0.0 Mobile Safari/537.36";
  const CHROME_ANDROID = "Mozilla/5.0 (Linux; Android 13; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36";
  const SAFARI_IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1";
  const tauri = { __TAURI_INTERNALS__: {}, __OE_ORIGINE__: "https://oe.example.com" };

  it("nelle app: Windows e Android, mai browser, anche con uno solo dei due segni dell'app", () => {
    expect(dovePagina(tauri, WEBVIEW2)).toBe("windows");
    expect(dovePagina(tauri, WEBVIEW_ANDROID)).toBe("android");
    expect(dovePagina({ __TAURI_INTERNALS__: {} }, WEBVIEW_ANDROID)).toBe("android");
    expect(dovePagina({ __OE_ORIGINE__: "https://oe.example.com" }, WEBVIEW2)).toBe("windows");
  });

  it("in un browser, anche sul telefono", () => {
    for (const ua of [WEBVIEW2, CHROME_ANDROID, SAFARI_IPHONE, ""]) expect(dovePagina({}, ua), ua).toBe("browser");
  });
});

describe("viste che scorrono", () => {
  it("riconosce il periodo, la cache e il bordo", () => {
    const i = new Date(2026, 9, 12);
    const f = new Date(2026, 9, 15);
    expect(aScorrimento(i, f)).toBe(true);
    expect(aScorrimento(i, new Date(2026, 9, 13))).toBe(true);
    expect(aScorrimento(i, new Date(2026, 9, 19))).toBe(true);
    expect(aScorrimento(i, new Date(2026, 9, 17))).toBe(false);
    expect(aScorrimento(new Date(2026, 8, 28), new Date(2026, 10, 9))).toBe(false);
    const c = periodoLargo(i, f);
    expect(dentroLaCache(c, i, f)).toBe(true);
    expect(dentroLaCache(c, spostaGiorni(i, 10), spostaGiorni(f, 10))).toBe(true);
    expect(dentroLaCache(c, spostaGiorni(i, 20), spostaGiorni(f, 20))).toBe(false);
    expect(dentroLaCache(null, i, f)).toBe(false);
    expect(vicinoAlBordo(c, i, f)).toBe(false);
    expect(vicinoAlBordo(c, spostaGiorni(i, 12), spostaGiorni(f, 12))).toBe(true);
  });

  it("al rilascio va al passo più vicino, più in là con un colpo veloce, mai oltre i giorni disegnati", () => {
    // colonne da 100 px, 3 giorni nascosti per lato, passo di un giorno; dx negativo = trascinata a sinistra = avanti
    expect(giorniDiArrivo(-40, 0, 100, 1, 3)).toBe(0);
    expect(giorniDiArrivo(40, 0, 100, 1, 3)).toBe(0);
    expect(giorniDiArrivo(-60, 0, 100, 1, 3)).toBe(1);
    expect(giorniDiArrivo(160, 0, 100, 1, 3)).toBe(-2);
    expect(giorniDiArrivo(-40, -1.5, 100, 1, 3)).toBe(3);
    expect(giorniDiArrivo(-290, -3, 100, 1, 3)).toBe(3);
    expect(giorniDiArrivo(30, 1, 100, 1, 3)).toBe(-2);
    // un trascinamento calmo di poco più di una colonna non va oltre il giorno dopo
    expect(giorniDiArrivo(-100, -0.5, 120, 1, 3)).toBe(1);
    // settimana: colonne da 50 px, passo e bordo di 7 giorni
    expect(giorniDiArrivo(-150, 0, 50, 7, 7)).toBe(0);
    expect(giorniDiArrivo(-200, 0, 50, 7, 7)).toBe(7);
    expect(giorniDiArrivo(60, 1.5, 50, 7, 7)).toBe(-7);
    expect(giorniDiArrivo(-10, 0, 0, 1, 3)).toBe(0);
  });
});

describe("timesheet", () => {
  it("una scheda con azione senza indirizzo non si mostra", () => {
    const s = schedeTimesheet([{ nome: "A", url: "https://a" }, { nome: "B", azione: "x" }, { nome: "C" }]);
    expect(s.map((x) => x.nome)).toEqual(["A", "C"]);
    expect(sigleScheda("Beta Servizi Srl")).toBe("BS");
  });
});

describe("client dell'API", () => {
  const risposta = (status: number, corpo: unknown) => new Response(JSON.stringify(corpo), { status });
  it("manda gli argomenti in una lista JSON a /api/<metodo>", async () => {
    let chiamata: { url: string; init: RequestInit } | null = null;
    const api = creaApi(async (url, init) => {
      chiamata = { url: String(url), init: init! };
      return risposta(200, { ok: true });
    });
    expect(await api("set_azienda_enabled", "ROAR", false)).toEqual({ ok: true });
    expect(chiamata!.url).toBe("/api/set_azienda_enabled");
    expect(chiamata!.init.method).toBe("POST");
    expect(JSON.parse(chiamata!.init.body as string)).toEqual(["ROAR", false]);
    expect(new Headers(chiamata!.init.headers).get("content-type")).toBe("application/json");
  });
  it("401: avvisa che la sessione è finita", async () => {
    let finita = 0;
    const api = creaApi(async () => risposta(401, { error: "non autenticato" }), () => finita++);
    await expect(api("get_config")).rejects.toBeInstanceOf(ApiError);
    expect(finita).toBe(1);
  });
  it("un errore del servizio porta il suo messaggio", async () => {
    const api = creaApi(async () => risposta(500, { error: "boom" }));
    await expect(api("get_config")).rejects.toThrow("boom");
    const senzaCorpo = creaApi(async () => new Response("<html>", { status: 502 }));
    await expect(senzaCorpo("get_config")).rejects.toThrow("HTTP 502");
  });
});

describe("vista adatta allo schermo", () => {
  it("su uno schermo stretto la settimana del computer si apre a tre giorni, il resto resta", () => {
    expect(vistaPerSchermo("timeGridWeek", false)).toBe("timeGridWeek");
    expect(vistaPerSchermo("timeGridWeek", true)).toBe("timeGridTreGiorni");
    for (const v of ["timeGridDay", "timeGridTreGiorni", "dayGridMonth", "listWeek"]) expect(vistaPerSchermo(v, true)).toBe(v);
  });
});

describe("urlLogo", () => {
  // un indirizzo di un altro sito non si carica dalla pagina: lo scarica il gateway (lib/core/loghi.ts) e alla pagina arriva il file in cache
  it.each([
    ["logos/serco.png", "/logos/serco.png"],
    ["/logos/serco.png", "/logos/serco.png"],
    ["https://esempio.example/logo.png", ""],
    ["//cdn.example/logo.png", ""],
  ])("%s -> %s", (valore, atteso) => expect(urlLogo(valore)).toBe(atteso));
});

describe("filtraAziende", () => {
  const az = (azienda: string, ...caselle: string[]) => ({ azienda, color: "#000", caselle, tot: 1, on: 1, rotto: false, avviso: false, motivi: [] });
  const tutte = [az("Serco", "antonio.maritato@serco.eu"), az("Prisma", "s.maritato@prismaprogetti.it"), az("Festività")];
  it("testo vuoto o di soli spazi: tutte, nello stesso ordine", () => {
    expect(filtraAziende(tutte, "")).toBe(tutte);
    expect(filtraAziende(tutte, "   ")).toBe(tutte);
  });
  it("cerca nel nome dell'azienda senza badare alle maiuscole", () => expect(filtraAziende(tutte, "SER").map((g) => g.azienda)).toEqual(["Serco"]));
  it("cerca anche nelle caselle", () => expect(filtraAziende(tutte, "prismaprogetti").map((g) => g.azienda)).toEqual(["Prisma"]));
  it("ignora gli accenti, in un senso e nell'altro", () => {
    expect(filtraAziende(tutte, "festivita").map((g) => g.azienda)).toEqual(["Festività"]);
    expect(filtraAziende(tutte, "festività").map((g) => g.azienda)).toEqual(["Festività"]);
  });
  it("niente che corrisponda: elenco vuoto", () => expect(filtraAziende(tutte, "zzz")).toEqual([]));
});

describe("separaSecondari", () => {
  const az = (azienda: string) => ({ azienda, color: "#000", caselle: [], tot: 1, on: 1, rotto: false, avviso: false, motivi: [] });
  const tutte = [az("Serco"), az("Personale"), az("Eventi personali"), az("Festività")];
  it("le secondarie vanno in coda al loro gruppo e ognuna tiene il suo ordine", () => {
    const [principali, altri] = separaSecondari(tutte, new Set(["Festività", "Eventi personali"]));
    expect(principali.map((g) => g.azienda)).toEqual(["Serco", "Personale"]);
    expect(altri.map((g) => g.azienda)).toEqual(["Eventi personali", "Festività"]);
  });
  it("senza secondarie, o con nomi che non ci sono, tutte restano principali", () => {
    expect(separaSecondari(tutte, new Set())).toEqual([tutte, []]);
    expect(separaSecondari(tutte, new Set(["Altro"]))).toEqual([tutte, []]);
  });
});

describe("limitaLarghezza", () => {
  it("tiene la larghezza fra il minimo e il massimo, in pixel interi", () => {
    expect(limitaLarghezza(300.4)).toBe(300);
    expect(limitaLarghezza(10)).toBe(LARGHEZZA_PANNELLO.min);
    expect(limitaLarghezza(9999)).toBe(LARGHEZZA_PANNELLO.max);
  });
  it("un valore che non è un numero torna a quello di partenza", () => {
    expect(limitaLarghezza(NaN)).toBe(LARGHEZZA_PANNELLO.predefinita);
    expect(limitaLarghezza(Infinity)).toBe(LARGHEZZA_PANNELLO.predefinita);
  });
});

describe("secondariEffettivi", () => {
  it("senza scelte valgono quelli del file", () => expect([...secondariEffettivi(["Festività"], undefined)]).toEqual(["Festività"]));
  it("la scelta della finestra mette e toglie, sopra il file", () => {
    expect([...secondariEffettivi(["Festività", "Eventi personali"], { Festività: false, Personale: true })].sort()).toEqual(["Eventi personali", "Personale"]);
  });
  it("niente file e niente scelte: nessuno", () => expect(secondariEffettivi(undefined, undefined).size).toBe(0));
});

describe("coloriDinamici", () => {
  let n = 0;
  const r = (alias: string, da: string, a: string, extra: Partial<Evento> = {}) =>
    ev({ id: `e${++n}`, alias, account: `${alias.toLowerCase()}@x.it`, start: `2026-10-13T${da}:00`, end: `2026-10-13T${a}:00`, ...extra });
  // otto tinte sulla ruota e, per ultimo, il grigio
  const [T0, T1, T2, , OPPOSTA, , T6, , GRIGIO] = TINTE_DINAMICHE;
  // la giornata del 13 ottobre 2026: nove aziende, cinque a contatto fra loro intorno alle 14:30
  const giornata = () => [
    r("Tesco", "10:40", "11:00"), r("Zurich", "10:45", "11:15"), r("Tesco", "11:00", "11:30"), r("ITV", "11:00", "11:15"), r("ITV", "11:15", "11:30"),
    r("BIP", "12:00", "13:00"), r("EVER", "12:00", "12:30"), r("ROAR", "12:00", "12:15"),
    r("Personale", "13:00", "14:00"), r("Zurich", "13:00", "13:30"),
    r("ITV", "14:00", "15:00"), r("EVER", "14:00", "14:30"), r("Personale", "14:30", "15:30"), r("MSC", "14:30", "15:30"), r("Tesco", "14:30", "15:00"),
    r("Tesco", "15:30", "16:00"), r("Notarify", "16:00", "16:30"),
  ];

  it("due aziende con riunioni sovrapposte prendono tinte opposte", () => {
    expect(coloriDinamici([r("Tesco", "09:00", "10:00"), r("ITV", "09:30", "10:30")], [])).toEqual(new Map([["ITV", T0], ["Tesco", OPPOSTA]]));
  });

  it("una riunione in fila all'altra conta, anche con cinque minuti di pausa", () => {
    const opposte = new Map([["ITV", T0], ["Tesco", OPPOSTA]]);
    expect(coloriDinamici([r("Tesco", "09:00", "10:00"), r("ITV", "10:00", "11:00")], [])).toEqual(opposte);
    expect(coloriDinamici([r("Tesco", "09:00", "09:55"), r("ITV", "10:00", "11:00")], [])).toEqual(opposte);
  });

  it("in cinque tutte a contatto: quattro tinte a un quarto di giro l'una dall'altra e il grigio", () => {
    const m = coloriDinamici(["A", "B", "C", "D", "E"].map((x) => r(x, "09:00", "10:00")), []);
    expect(new Set(m.values())).toEqual(new Set([T0, T2, OPPOSTA, T6, GRIGIO]));
  });

  it("nella giornata piena nessuna coppia di aziende vicine ha la stessa tinta", () => {
    const eventi = giornata();
    const m = coloriDinamici(eventi, []);
    const quarto = 15 * 60000;
    for (const a of eventi)
      for (const b of eventi) {
        if (a.alias === b.alias) continue;
        const vicine = Date.parse(a.start) <= Date.parse(b.end) + quarto && Date.parse(b.start) <= Date.parse(a.end) + quarto;
        if (vicine) expect(m.get(a.alias!), `${a.alias} e ${b.alias}`).not.toBe(m.get(b.alias!));
      }
  });

  it("riletta la stessa pagina i colori restano quelli, in qualunque ordine arrivino gli eventi", () => {
    const m = coloriDinamici(giornata(), []);
    expect(coloriDinamici(giornata(), [], m)).toEqual(m);
    expect(coloriDinamici(giornata().reverse(), [])).toEqual(m);
  });

  it("chi ha già una tinta la tiene finché quella accanto non è uguale o vicina", () => {
    const eventi = [r("Tesco", "09:00", "10:00"), r("ITV", "09:30", "10:30")];
    expect(coloriDinamici(eventi, [], new Map([["Tesco", T0], ["ITV", T2]]))).toEqual(new Map([["Tesco", T0], ["ITV", T2]]));
    expect(coloriDinamici(eventi, [], new Map([["Tesco", T0], ["ITV", T1]]))).toEqual(new Map([["ITV", T1], ["Tesco", TINTE_DINAMICHE[5]]]));
    expect(coloriDinamici(eventi, [], new Map([["Tesco", T0], ["ITV", T0]]))).toEqual(new Map([["ITV", T0], ["Tesco", OPPOSTA]]));
  });

  it("due aziende lontane un'ora possono avere la stessa tinta", () => {
    const stesse = new Map([["Tesco", T0], ["ITV", T0]]);
    expect(coloriDinamici([r("Tesco", "09:00", "10:00"), r("ITV", "11:00", "12:00")], [], stesse)).toEqual(stesse);
  });

  it("le aziende dell'elenco hanno una tinta anche senza riunioni; festività e calendari spenti non contano", () => {
    const stesse = new Map([["Tesco", T0], ["ITV", T0]]);
    const m = coloriDinamici(
      [r("Tesco", "09:00", "10:00"), r("ITV", "09:30", "10:30", { nascosto: true }), r("Festività", "00:00", "23:59", { account: "Festività", allDay: true })],
      ["Serco", "ITV", "Tesco"],
      stesse,
    );
    expect([...m.keys()].sort()).toEqual(["ITV", "Serco", "Tesco"]);
    expect(TINTE_DINAMICHE).toContain(m.get("Serco"));
    expect(m.get("ITV")).toBe(T0);
    expect(m.get("Tesco")).toBe(T0);
  });

  it("contano solo le riunioni della pagina che si guarda", () => {
    const stesse = new Map([["Tesco", T0], ["ITV", T0]]);
    const eventi = [r("Tesco", "09:00", "10:00"), r("ITV", "09:30", "10:30")];
    const domani = { start: new Date(2026, 9, 14), end: new Date(2026, 9, 15) };
    expect(coloriDinamici(eventi, ["Tesco", "ITV"], stesse, domani)).toEqual(stesse);
    expect(coloriDinamici(eventi, ["Tesco", "ITV"], stesse, { start: new Date(2026, 9, 13), end: new Date(2026, 9, 14) })).not.toEqual(stesse);
  });

  it("con più aziende a contatto che tinte, la ripetizione tocca alla coppia che si incontra meno", () => {
    const tutte = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "Z"];
    expect(tutte.length).toBe(TINTE_DINAMICHE.length + 1);
    const m = coloriDinamici(
      [
        ...tutte.map((x) => r(x, "09:00", "10:00")),
        ...tutte.filter((x) => x !== "A").map((x) => r(x, "11:00", "12:00")),
        ...tutte.filter((x) => x !== "Z").map((x) => r(x, "13:00", "14:00")),
      ],
      [],
    );
    expect(new Set(m.values()).size).toBe(TINTE_DINAMICHE.length);
    expect(m.get("Z")).toBe(m.get("A"));
  });
});

describe("impostazioni del dispositivo", () => {
  const server = { theme: "dark" as const, zoom_calendario: 70, disabled_calendars: ["a"], gruppi_calendari: { G: ["Alfa"] }, account_aliases: { x: "X" } };
  it("un dispositivo senza impostazioni parte da quelle del server; uno con le sue tiene le sue", () => {
    expect(unisciConfig(server, null)).toEqual({ config: server, dispositivo: { theme: "dark", zoom_calendario: 70, disabled_calendars: ["a"] } });
    const mio = { theme: "light" as const, disabled_calendars: [] };
    expect(unisciConfig(server, mio).config).toEqual({ ...server, ...mio });
  });
  it("una modifica si divide tra dispositivo e server", () => {
    const patch = { vista: "timeGridDay", colori_dinamici: true, nascondi_tutto_il_giorno: true, gruppi_calendari: {} };
    expect(soloDelDispositivo(patch)).toEqual({ vista: "timeGridDay", colori_dinamici: true, nascondi_tutto_il_giorno: true });
    expect(soloDelServer(patch)).toEqual({ gruppi_calendari: {} });
  });
  it("cambiaSpenti accende, spegne e lascia com'è", () => {
    const cals = ["a", "b", "c"].map((id) => ({ id, account: id, label: id, color: "", enabled: true }));
    expect(cambiaSpenti(cals, ["a", "z"], (c) => (c.id === "a" ? true : c.id === "c" ? false : undefined))).toEqual(["c", "z"]);
  });
});

describe("dentro l'app", () => {
  it("nel browser non c'è niente: null", () => {
    expect(infoApp({})).toBeNull();
    expect(infoApp({ __OE_ORIGINE__: "https://oe.prova.test" })).toBeNull();
  });
  it("l'app dà l'indirizzo del sito e quello della pagina di avvio: host e link per cambiare server", () => {
    expect(infoApp({ __OE_ORIGINE__: "https://oe.prova.test:8443", __OE_AVVIO__: "http://tauri.localhost/index.html" })).toEqual({
      host: "oe.prova.test:8443",
      cambiaServer: "http://tauri.localhost/index.html#server",
    });
  });
  it("valori che non sono indirizzi: null", () => {
    expect(infoApp({ __OE_ORIGINE__: 5, __OE_AVVIO__: "http://tauri.localhost/" })).toBeNull();
    expect(infoApp({ __OE_ORIGINE__: "non un indirizzo", __OE_AVVIO__: "http://tauri.localhost/" })).toBeNull();
    expect(infoApp({ __OE_ORIGINE__: "https://oe.prova.test", __OE_AVVIO__: "" })).toBeNull();
  });
});
