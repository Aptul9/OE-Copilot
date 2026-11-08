// Le serie di eventi con le loro eccezioni, come le calcola recurring_ical_events 3.8.2 (Series, RecurrenceRules, ComponentAdapter):
// un evento "nucleo" con le sue regole, le modifiche (RECURRENCE-ID) che prendono il posto di un'occorrenza, le date escluse e aggiunte.
// Ogni scelta qui ha un'origine nella libreria Python, comprese le stranezze: le prove confrontano i risultati con le sue risposte.

import { type Naive, type Regola, chiaveMuro, iteraRegola, leggiRegola } from "./regola.ts";
import {
  type Ora,
  type Tempo,
  ErroreValore,
  comeData,
  comeOra,
  confronta,
  confrontaGrande,
  confrontaT,
  data,
  differenzaT,
  idRicorrenza,
  inFuso,
  istante,
  ora,
  rendiComparabili,
  somma,
  sommaT,
  testoPy,
} from "./tempo.ts";

export interface Dati {
  uid: string;
  titolo: string;
  luogo: string;
  descrizione: string;
  organizzatore: string;
}

export type Rdate = { inizio: Tempo; fineOdurata: Tempo | number | null };

/** Un VEVENT: le proprietà che servono al calcolo delle ricorrenze, più quelle che l'agente restituisce. */
export interface Componente {
  uid: string;
  inizio: Tempo;
  fineProp: Tempo | null;
  durata: number | null;
  recId: Tempo | null;
  recRange: boolean;
  sequenza: number;
  exdate: Tempo[];
  rdate: Rdate[];
  rrule: string[];
  haRegole: boolean;
  /** Gli errori di proprietà rotte, nell'ordine in cui il calcolo di Python le incontra: il primo esce quando si costruisce la serie. */
  problemi?: Error[];
  dati: Dati;
}

export interface Occorrenza {
  c: Componente;
  inizio: Tempo;
  fine: Tempo;
}

const secondiDelGiorno = (d: number) => ((d % 86400) + 86400) % 86400;
const dopo = (t: Tempo, secondi: number): Tempo => sommaT(t, secondi);
/** data - timedelta: a una data si tolgono solo i giorni interi della durata. */
const prima = (t: Tempo, secondi: number): Tempo => (t.tipo === "data" ? data(t.d.subtract({ days: Math.floor(secondi / 86400) })) : somma(t, -secondi));

/** time_span_contains_event: gli estremi finali sono esclusi, quelli iniziali compresi; un evento senza durata conta nell'istante in cui cade. */
export function contieneEvento(inizioFinestra: Tempo, fineFinestra: Tempo, inizioEvento: Tempo, fineEvento: Tempo): boolean {
  const [a, b, c, d] = rendiComparabili([inizioFinestra, fineFinestra, inizioEvento, fineEvento]) as [Tempo, Tempo, Tempo, Tempo];
  if (confrontaT(c, d) > 0) throw new ErroreValore(`the event must start before it ends(start: ${testoPy(c)} end: ${testoPy(d)})`);
  if (confrontaT(a, b) > 0) throw new ErroreValore(`the time span must start before it ends(start: ${testoPy(a)} end: ${testoPy(b)})`);
  if (confrontaT(c, d) === 0) {
    if (confrontaT(a, b) === 0) return confrontaT(c, a) === 0;
    return confrontaT(a, c) <= 0 && confrontaT(c, b) < 0;
  }
  if (confrontaT(a, b) === 0) return confrontaT(c, a) <= 0 && confrontaT(a, d) < 0;
  return confrontaT(c, b) < 0 && confrontaT(a, d) < 0;
}

class Adattatore {
  private _span: [Tempo, Tempo] | null = null;
  readonly c: Componente;
  constructor(c: Componente) {
    this.c = c;
  }

  get fineGrezza(): Tempo {
    const c = this.c;
    if (c.fineProp) return c.fineProp;
    if (c.durata !== null) {
      const inizio = secondiDelGiorno(c.durata) !== 0 && c.inizio.tipo === "data" ? comeOra(c.inizio, null) : c.inizio;
      return dopo(inizio, c.durata);
    }
    return c.inizio.tipo === "data" ? dopo(c.inizio, 86400) : c.inizio;
  }

  get span(): [Tempo, Tempo] {
    if (!this._span) {
      const [inizio, fine] = rendiComparabili([this.c.inizio, this.fineGrezza]) as [Tempo, Tempo];
      this._span = confrontaT(inizio, fine) > 0 ? [fine, inizio] : [inizio, fine];
    }
    return this._span;
  }
  get inizio() {
    return this.span[0];
  }
  get fine() {
    return this.span[1];
  }
  get durata() {
    return differenzaT(this.fine, this.inizio);
  }
  get sequenza() {
    return this.c.sequenza;
  }
  get idRic(): string[] {
    return this.c.recId ? idRicorrenza(this.c.recId) : [];
  }
  get èModifica() {
    return this.idRic.length > 0;
  }

  /** Quanto spostare le occorrenze: solo per RANGE=THISANDFUTURE, la differenza fra il nuovo inizio e RECURRENCE-ID. */
  get spostamento(): number {
    if (!this.c.recRange || !this.c.recId) return 0;
    const [inizio, id] = rendiComparabili([this.inizio, this.c.recId]) as [Tempo, Tempo];
    return differenzaT(inizio, id);
  }

  /** Di quanto allargare la finestra delle regole per non perdere le occorrenze lunghe o spostate: (indietro, avanti). */
  get estensione(): [number, number] {
    let indietro = this.durata;
    let avanti = 0;
    if (this.c.recId) {
      const [inizio, fine, id] = rendiComparabili([this.inizio, this.fine, this.c.recId]) as [Tempo, Tempo, Tempo];
      if (confrontaT(inizio, id) < 0) avanti = differenzaT(id, inizio);
      if (confrontaT(inizio, id) > 0) indietro = differenzaT(fine, id);
    }
    return [indietro, avanti];
  }
}

const piuSequenza = (a: Adattatore | null | undefined, b: Adattatore) => (!a || b.sequenza > a.sequenza ? b : a);

interface RegolaUntil {
  regola: Regola;
  until: Tempo | null;
}

/** RecurrenceRules: il nucleo della serie con le sue date escluse e aggiunte e le sue regole, tutto confrontabile fra loro. */
class Ricorrenze {
  readonly inizioOrig: Tempo;
  readonly fineOrig: Tempo;
  readonly tz: string | null;
  readonly tutteDate: boolean;
  readonly inizio: Ora;
  readonly fine: Ora;
  readonly chkOra = new Set<string>();
  readonly chkData = new Set<string>();
  readonly sostituisciFine = new Map<string, number>();
  readonly regole: RegolaUntil[] = [];
  readonly insieme: Ora[] = [];

  readonly nucleo: Adattatore;

  constructor(nucleo: Adattatore) {
    this.nucleo = nucleo;
    this.inizioOrig = nucleo.inizio;
    this.fineOrig = nucleo.fine;
    for (const e of nucleo.c.exdate) {
      for (const id of idRicorrenza(e)) this.chkOra.add(id);
      if (e.tipo === "data") this.chkData.add(e.d.toString());
    }
    const rdate: Tempo[] = [];
    for (const r of nucleo.c.rdate) {
      rdate.push(r.inizio);
      if (r.fineOdurata !== null) {
        const d = typeof r.fineOdurata === "number" ? r.fineOdurata : differenzaT(...(rendiComparabili([r.fineOdurata, r.inizio]) as [Tempo, Tempo]));
        for (const id of idRicorrenza(r.inizio)) this.sostituisciFine.set(id, d);
      }
    }
    // make_all_dates_comparable: il fuso della serie è il primo che si trova fra inizio, fine, esclusioni e aggiunte
    const tutti = [this.inizioOrig, this.fineOrig, ...nucleo.c.exdate, ...rdate];
    this.tutteDate = !tutti.some((t) => t.tipo === "ora");
    let tz: string | null = null;
    for (const t of tutti) {
      if (t.tipo === "ora" && t.z !== null) {
        tz = t.z;
        break;
      }
    }
    this.tz = tz;
    this.inizio = comeOra(this.inizioOrig, tz);
    this.fine = comeOra(this.fineOrig, tz);
    const aggiunte = rdate.map((t) => comeOra(t, tz));

    let ultimoUntil: Tempo | null = null;
    for (const testo of nucleo.c.rrule) {
      const regola = this.creaRegola(testo);
      this.regole.push(regola);
      if (regola.until && (!ultimoUntil || confrontaGrande(regola.until, ultimoUntil))) ultimoUntil = regola.until;
    }
    this.insieme.push(...aggiunte);
    if (!ultimoUntil || !confrontaGrande(this.inizio, ultimoUntil)) this.insieme.push(this.inizio);
    this.insieme.sort(confronta);
  }

  private muro(o: Ora): Naive {
    return { a: o.w.year, m: o.w.month, g: o.w.day, h: o.w.hour, mi: o.w.minute, s: o.w.second };
  }

  /** create_rule_with_start: UNTIL deve avere la stessa natura di DTSTART (con o senza fuso); se no si adatta, come fa la libreria. */
  private creaRegola(testo: string): RegolaUntil {
    let regola: Regola;
    try {
      regola = leggiRegola(testo, this.muro(this.inizio));
    } catch (e) {
      if (!(e instanceof ErroreValore)) throw e;
      // create_rule_with_start: dateutil rifiuta la regola, si riprova con UNTIL adattato a DTSTART; senza UNTIL (o con due) è un errore
      const parti = testo.split(";UNTIL=");
      if (parti.length !== 2) throw new ErroreValore(`UNTIL parameter is missing: ${testo}`);
      let fineData = parti[1]!.indexOf(";");
      if (fineData === -1) fineData = parti[1]!.length;
      let until = parti[1]!.slice(0, fineData);
      if (this.tutteDate) until = until.slice(0, 8);
      else if (this.tz === null) until = until.slice(0, -1);
      else {
        if (until.length === 8) until += "T000000";
        if (until.length !== 15) throw new ErroreValore(`UNTIL parameter has a bad format: ${testo}`);
        until += "Z";
      }
      regola = leggiRegola(`${parti[0]}${parti[1]!.slice(fineData)};UNTIL=${until}`, this.muro(this.inizio));
    }
    let until: Tempo | null = null;
    const u = regola.until;
    if (u !== null) {
      const parti = (s: string) => Temporal.PlainDateTime.from(`${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}T${s.length > 8 ? `${s.slice(9, 11)}:${s.slice(11, 13)}:${s.slice(13, 15)}` : "00:00:00"}`);
      try {
        if (this.tz !== null) until = ora(parti(u.length === 8 ? `${u}T000000` : u.slice(0, 15)), "UTC");
        else if (this.tutteDate) until = u.length === 15 ? ora(parti(u)) : data(parti(u.slice(0, 8)).toPlainDate());
        else until = u.length === 8 ? data(parti(u).toPlainDate()) : ora(parti(u.slice(0, 15)));
      } catch (e) {
        throw new ErroreValore(`UNTIL non valido: ${u}`, { cause: e });
      }
    }
    return { regola, until };
  }

  private dopoUntil(r: RegolaUntil): ((n: Naive) => boolean) | null {
    const until = r.until;
    if (!until) return null;
    const u = comeOra(until, until.tipo === "ora" ? until.z : null);
    return (n) => confronta(this.comeOraDi(n), u) > 0;
  }

  private comeOraDi(n: Naive): Ora {
    return ora(Temporal.PlainDateTime.from({ year: n.a, month: n.m, day: n.g, hour: n.h, minute: n.mi, second: n.s }), this.tz);
  }

  /** La chiave sul muro di un estremo, nel fuso della serie: serve a saltare in fretta i risultati lontani dalla finestra. */
  private chiaveEstremo(o: Ora): number {
    if (this.tz === null || o.z === this.tz) return chiaveMuro(this.muro(o));
    return chiaveMuro(this.muro(inFuso(istante(o), this.tz)));
  }

  *tra(inizioFinestra: Tempo, fineFinestra: Tempo): Generator<Ora> {
    const a = comeOra(inizioFinestra, this.tz);
    const b = comeOra(fineFinestra, this.tz);
    for (const x of this.insieme) if (confronta(x, a) >= 0 && confronta(x, b) <= 0) yield x;
    const MARGINE = 3 * 86400;
    for (const r of this.regole) {
      const da = this.chiaveEstremo(a) - MARGINE;
      const fino = this.chiaveEstremo(b) + MARGINE;
      let iniziato = false;
      for (const n of iteraRegola(r.regola, this.muro(this.inizio), this.dopoUntil(r))) {
        const k = chiaveMuro(n);
        if (k < da) continue;
        if (k > fino) break;
        const x = this.comeOraDi(n);
        if (confronta(x, b) > 0) break;
        if (!iniziato) {
          if (confronta(x, a) < 0) continue;
          iniziato = true;
        }
        if (r.until === null || !confrontaGrande(x, r.until)) yield x;
      }
    }
  }

  /** Il valore come l'aveva scritto il calendario: una data se inizio e fine lo erano. */
  nelTipoOriginale(t: Ora): Tempo {
    return this.inizioOrig.tipo === "data" && this.fineOrig.tipo === "data" ? data(comeData(t)) : t;
  }
}

/** Una serie: tutti i VEVENT con lo stesso UID. */
export class Serie {
  private readonly modifiche = new Map<string, Adattatore>();
  private readonly elencoModifiche: Adattatore[];
  private readonly daQuiInPoi: string[] = [];
  private nucleo: Adattatore | null = null;
  private readonly ricorrenze: Ricorrenze | null;
  private indietro: number;
  private avanti: number;

  constructor(componenti: Componente[]) {
    if (!componenti.length) throw new ErroreValore("No components given to calculate a series.");
    for (const c of componenti) {
      const ad = new Adattatore(c);
      if (ad.èModifica) {
        for (const id of ad.idRic) this.modifiche.set(id, piuSequenza(this.modifiche.get(id), ad));
        if (c.recRange) this.daQuiInPoi.push(ad.idRic[0]!);
      } else this.nucleo = piuSequenza(this.nucleo, ad);
    }
    this.elencoModifiche = [...new Set(this.modifiche.values())];
    this.daQuiInPoi.sort();
    this.ricorrenze = this.nucleo ? new Ricorrenze(this.nucleo) : null;
    [this.indietro, this.avanti] = this.nucleo ? this.nucleo.estensione : [0, 0];
    for (const id of this.daQuiInPoi) {
      const [i, a] = this.modifiche.get(id)!.estensione;
      this.indietro = Math.max(this.indietro, i);
      this.avanti = Math.max(this.avanti, a);
    }
  }

  private componentePer(id: string): Adattatore {
    let componente = this.nucleo!;
    for (const m of this.daQuiInPoi) {
      if (m < id) componente = this.modifiche.get(m)!;
      else break;
    }
    return componente;
  }

  private *regoleTra(inizio: Tempo, fine: Tempo): Generator<Ora> {
    if (this.ricorrenze) yield* this.ricorrenze.tra(prima(inizio, this.indietro), dopo(fine, this.avanti));
  }

  private escluso(ids: string[]): boolean {
    return !!this.ricorrenze && ids.some((id) => this.ricorrenze!.chkOra.has(id));
  }

  /** Le occorrenze che si sovrappongono all'intervallo, nell'ordine in cui le dà la libreria. */
  tra(inizio: Tempo, fine: Tempo): Occorrenza[] {
    const out: Occorrenza[] = [];
    const visti: Ora[] = [];
    const modificheDate = new Set<Adattatore>();
    const occ = (a: Adattatore): Occorrenza => ({ c: a.c, inizio: a.inizio, fine: a.fine });
    for (const s of this.regoleTra(inizio, fine)) {
      const ids = idRicorrenza(s);
      if (visti.some((v) => confronta(v, s) === 0) || this.ricorrenze!.chkData.has(comeData(s).toString()) || this.escluso(ids)) continue;
      let ad: Adattatore = this.nucleo!;
      for (const id of ids) ad = this.modifiche.get(id) ?? ad;
      let o: Occorrenza;
      if (ad === this.nucleo) {
        visti.push(s);
        const componente = this.componentePer(ids[0]!);
        const inizioOcc = dopo(s, componente.spostamento) as Ora;
        let durata = componente.durata;
        for (const id of ids) durata = this.ricorrenze!.sostituisciFine.get(id) ?? durata;
        const fineOcc = dopo(inizioOcc, durata) as Ora;
        o = { c: componente.c, inizio: this.ricorrenze!.nelTipoOriginale(inizioOcc), fine: this.ricorrenze!.nelTipoOriginale(fineOcc) };
      } else {
        if (modificheDate.has(ad)) continue;
        modificheDate.add(ad);
        o = occ(ad);
      }
      if (contieneEvento(inizio, fine, o.inizio, o.fine)) out.push(o);
    }
    for (const m of this.elencoModifiche) {
      if (modificheDate.has(m) || this.escluso(m.idRic) || this.saltaModifica(m)) continue;
      if (contieneEvento(inizio, fine, m.inizio, m.fine)) {
        modificheDate.add(m);
        out.push(occ(m));
      }
    }
    return out;
  }

  /** skip_core_modification: una modifica con regole proprie e SEQUENCE più vecchia del nucleo vale solo se la regola del nucleo produce quella data. */
  private saltaModifica(m: Adattatore): boolean {
    if (!m.c.haRegole || !m.èModifica) return false;
    const sequenzaNucleo = this.nucleo ? this.nucleo.sequenza : -1;
    if (m.sequenza >= sequenzaNucleo) return false;
    // convert_to_date_range del primo id (l'istante UTC senza fuso): il giorno intero, letto nel fuso della serie
    const inizio = ora(Temporal.PlainDateTime.from(m.idRic[0]!).with({ hour: 0, minute: 0, second: 0 }));
    for (const s of this.regoleTra(inizio, dopo(inizio, 86400))) {
      const ids = idRicorrenza(s);
      if (this.ricorrenze!.chkData.has(comeData(s).toString()) || this.escluso(ids)) continue;
      if (ids.some((x) => m.idRic.includes(x))) return false;
    }
    return true;
  }
}

/** Le occorrenze di tutte le serie di un calendario nell'intervallo, con le serie nell'ordine in cui compaiono i loro UID. */
export function espandi(componenti: Componente[], inizio: Tempo, fine: Tempo): Occorrenza[] {
  const perUid = new Map<string, Componente[]>();
  for (const c of componenti) {
    const lista = perUid.get(c.uid);
    if (lista) lista.push(c);
    else perUid.set(c.uid, [c]);
  }
  // recurring_ical_events costruisce tutte le serie prima di chiedere la finestra: un errore di costruzione esce prima di ogni calcolo
  const serie = [...perUid.values()].map((lista) => {
    for (const c of lista) if (c.problemi?.length) throw c.problemi[0];
    return new Serie(lista);
  });
  const out: Occorrenza[] = [];
  for (const s of serie) out.push(...s.tra(inizio, fine));
  return out;
}

