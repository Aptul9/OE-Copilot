"use client";

import { useTheme } from "next-themes";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/oe/api";
import {
  cambiaSpenti,
  coloriDinamici,
  dentroLaCache,
  aScorrimento,
  etichettaSync,
  eventiPerGriglia,
  gruppiAttivi,
  inOraDelDispositivo,
  isoLocale,
  leggiGruppi,
  periodoLargo,
  raccogliFestivi,
  raggruppaAziende,
  secondariEffettivi,
  soloDelDispositivo,
  soloDelServer,
  testoDatiVecchi,
  unisciConfig,
  vicinoAlBordo,
  type CacheLarga,
  type EventoGriglia,
  type GruppoAzienda,
  type InfoGiorni,
} from "@/lib/oe/logic";
import type { Calendario, Config, Evento, EventiRisposta, FonteStato, GruppiCalendari, Personalizzazione, TemaConfig } from "@/lib/oe/types";

export type Periodo = { start: Date; end: Date; tipo: string };
export type Avviso = { testo: string; tipo: "errore" | "info" };

export const ZOOM_MIN = 26;
export const ZOOM_MAX = 110;
export const ZOOM_PASSO = 8;
export const ZOOM_DEFAULT = 58;
const MAX_TENTATIVI = 12;   // ~2 minuti: quanto basta a un riavvio del server dei calendari
const CHIAVE_DISPOSITIVO = "oe.impostazioni-dispositivo";

/** Le impostazioni di questo dispositivo (VOCI_DEL_DISPOSITIVO in logic.ts), o null se non ne ha ancora. */
function leggiDispositivo(): Partial<Config> | null {
  try {
    const v = JSON.parse(localStorage.getItem(CHIAVE_DISPOSITIVO) ?? "null");
    return v && typeof v === "object" && !Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}
function scriviDispositivo(c: Partial<Config>): void {
  try {
    localStorage.setItem(CHIAVE_DISPOSITIVO, JSON.stringify(c));
  } catch {
    // vale in questa pagina e non resta
  }
}

type Valori = {
  /** La pagina pubblica (senza accesso): stessi dati e stesse viste, impegni ridotti a "Occupato", niente scritture. */
  pubblico: boolean;
  gruppoPubblico: string;
  pronta: boolean;
  pers: Personalizzazione;
  /** Rilegge la personalizzazione (i loghi scelti dalla finestra Calendari ci stanno sopra). */
  ricaricaPers: () => Promise<void>;
  config: Config;
  vistaIniziale: string | null;
  // calendari e legenda
  calendari: Calendario[];
  gruppi: GruppoAzienda[];
  statoFonti: Record<string, FonteStato>;
  pallinoInAttesa: boolean;
  aggiornaCalendari: (forza?: boolean) => Promise<void>;
  accendiAzienda: (azienda: string, acceso: boolean) => Promise<void>;
  accendiCalendario: (id: string, acceso: boolean) => Promise<void>;
  soloAzienda: (azienda: string) => Promise<void>;
  // gruppi di calendari
  gruppiCalendari: GruppiCalendari;
  gruppiAccesi: string[];
  soloGruppo: (gruppo: string) => Promise<void>;
  salvaGruppi: (gruppi: GruppiCalendari) => Promise<void>;
  secondari: ReadonlySet<string>;
  impostaSecondario: (nome: string, secondario: boolean) => Promise<void>;
  // eventi
  periodo: Periodo | null;
  impostaPeriodo: (p: Periodo) => void;
  eventi: Evento[];
  eventiGriglia: EventoGriglia[];
  infoGiorni: InfoGiorni;
  ricerca: string;
  impostaRicerca: (q: string) => void;
  ricarica: () => void;
  forzaSync: () => void;
  // stato della sincronizzazione
  avviso: Avviso | null;
  datiVecchi: string | null;
  sincronizzando: boolean;
  etichetta: string;
  // impostazioni
  salvaConfig: (patch: Partial<Config>) => Promise<void>;
  zoom: number;
  cambiaZoom: (verso: 1 | -1 | 0) => void;
  intervalloSync: number;
  impostaIntervallo: (secondi: number) => void;
  tema: TemaConfig;
  temaScuro: boolean;
  impostaTema: (t: TemaConfig) => void;
  mostraAvviso: (a: Avviso | null) => void;
};

const Contesto = createContext<Valori | null>(null);

export function useOe(): Valori {
  const v = useContext(Contesto);
  if (!v) throw new Error("useOe fuori da OeProvider");
  return v;
}

export function OeProvider({ children, pubblico }: { children: React.ReactNode; pubblico?: { gruppo: string } }) {
  const inPubblico = !!pubblico;
  const gruppoPubblico = pubblico?.gruppo ?? "";
  const { setTheme, theme, resolvedTheme } = useTheme();
  const [pronta, setPronta] = useState(false);
  const [pers, setPers] = useState<Personalizzazione>({ interfaccia: {} });
  const [config, setConfig] = useState<Config>({});
  const [vistaIniziale, setVistaIniziale] = useState<string | null>(null);
  const [calendariLetti, setCalendari] = useState<Calendario[]>([]);
  const [statoFonti, setStatoFonti] = useState<Record<string, FonteStato>>({});
  const [pallinoInAttesa, setPallinoInAttesa] = useState(false);
  const [periodo, impostaPeriodo] = useState<Periodo | null>(null);
  const [eventiLetti, setEventi] = useState<Evento[]>([]);
  const [ricerca, impostaRicerca] = useState("");
  const [avviso, mostraAvviso] = useState<Avviso | null>(null);
  const [datiVecchi, setDatiVecchi] = useState<string | null>(null);
  const [inCorso, setInCorso] = useState(0);
  const [ultimaSync, setUltimaSync] = useState<number | null>(null);
  const [adesso, setAdesso] = useState(() => Date.now());
  const [token, setToken] = useState(0);
  const [zoom, setZoom] = useState(ZOOM_DEFAULT);
  const [intervalloSync, setIntervallo] = useState(60);

  const cache = useRef<CacheLarga | null>(null);
  const richiesta = useRef(0);
  const tentativi = useRef(0);
  const timerRitentativo = useRef<ReturnType<typeof setTimeout> | null>(null);
  const spentiRef = useRef<string[]>([]);
  const eraFerma = useRef(false);
  const inCorsoRef = useRef(0);
  const rabboccoInCorso = useRef(false);
  inCorsoRef.current = inCorso;

  // ---- avvio: personalizzazione e configurazione prima di disegnare ------------------
  useEffect(() => {
    if (inPubblico) {
      setPronta(true);
      return;
    }
    let vivo = true;
    Promise.all([
      api("get_personalizzazione").catch(() => ({ interfaccia: {} }) as Personalizzazione),
      api("get_config").catch(() => null),
    ]).then(([p, server]) => {
      if (!vivo) return;
      setPers(p);
      // senza la config del server (niente rete e niente copia) vale quella del dispositivo, e non si prende come punto di partenza
      const salvata = leggiDispositivo();
      const { config: c, dispositivo } = unisciConfig(server ?? {}, salvata);
      if (server && !salvata) scriviDispositivo(dispositivo);
      spentiRef.current = Array.isArray(c.disabled_calendars) ? c.disabled_calendars : [];
      setConfig(c);
      setVistaIniziale(typeof c.vista === "string" ? c.vista : null);
      if (c.theme) setTheme(c.theme === "auto" ? "system" : c.theme);
      if (c.zoom_calendario) setZoom(Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(c.zoom_calendario))));
      if (c.reminder_poll_seconds) setIntervallo(c.reminder_poll_seconds);
      setPronta(true);
    });
    return () => {
      vivo = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const ricaricaPers = useCallback(async () => {
    try {
      setPers(await api("get_personalizzazione"));
    } catch {
      // resta quella di prima
    }
  }, []);

  // I loghi scritti come indirizzo in personalizzazione.json il server li scarica in secondo piano: finché mancano si rilegge ogni 6 secondi, al più 5 volte
  const riletture = useRef(0);
  useEffect(() => {
    if (!pers.interfaccia.loghi_in_corso?.length) {
      riletture.current = 0;
      return;
    }
    if (riletture.current >= 5) return;
    const t = setTimeout(() => {
      riletture.current++;
      void ricaricaPers();
    }, 6000);
    return () => clearTimeout(t);
  }, [pers, ricaricaPers]);

  // ---- sincronizzazione: contatore e orologio dell'etichetta ---------------------------
  const syncInizio = useCallback(() => setInCorso((n) => n + 1), []);
  const syncFine = useCallback((riuscito: boolean) => {
    setInCorso((n) => Math.max(0, n - 1));
    if (riuscito) setUltimaSync(Date.now());
  }, []);
  useEffect(() => {
    const t = setInterval(() => setAdesso(Date.now()), 15000);
    return () => clearInterval(t);
  }, []);
  useEffect(() => {
    if (inCorso === 0) setAdesso(Date.now());
  }, [inCorso]);

  // ---- calendari -----------------------------------------------------------------------
  const ricarica = useCallback(() => {
    cache.current = null;   // una rilettura voluta scarta la memoria della vista a 3 giorni
    setToken((t) => t + 1);
  }, []);

  const aggiornaCalendariRef = useRef<(forza?: boolean) => Promise<void>>(async () => {});

  const pianificaRitentativo = useCallback(() => {
    if (timerRitentativo.current || tentativi.current >= MAX_TENTATIVI) return;
    tentativi.current++;
    timerRitentativo.current = setTimeout(() => {
      timerRitentativo.current = null;
      // quando il server risponde di nuovo vanno recuperati entrambi: l'elenco e gli eventi
      void aggiornaCalendariRef.current();
      ricarica();
    }, 10000);
  }, [ricarica]);

  const aggiornaCalendari = useCallback(
    async (forza = false) => {
      if (inPubblico) return;
      try {
        const res = await api("get_calendars", forza);
        if (res.error) {
          // Tipicamente il server dei calendari non risponde ancora: non si svuota l'elenco, si riprova da soli.
          pianificaRitentativo();
          if (!res.calendars.length) return;
        }
        setCalendari(res.calendars);
      } catch (e) {
        mostraAvviso({ testo: `Calendari non caricati: ${e instanceof Error ? e.message : String(e)}`, tipo: "errore" });
      }
    },
    [pianificaRitentativo, inPubblico],
  );
  aggiornaCalendariRef.current = aggiornaCalendari;

  // il service worker (public/sw.js) tiene la pagina e le ultime letture: senza rete la pagina si apre con quelle
  useEffect(() => {
    if (!inPubblico) navigator.serviceWorker?.register("/sw.js").catch(() => {});
  }, [inPubblico]);

  // tornata la rete si rilegge subito, senza aspettare il prossimo tentativo; lo stesso quando il service worker prende la pagina
  // (la prima volta arriva a letture già fatte, che quindi non ha salvato)
  useEffect(() => {
    const subito = () => {
      void aggiornaCalendariRef.current();
      ricarica();
    };
    window.addEventListener("online", subito);
    navigator.serviceWorker?.addEventListener("controllerchange", subito);
    return () => {
      window.removeEventListener("online", subito);
      navigator.serviceWorker?.removeEventListener("controllerchange", subito);
    };
  }, [ricarica]);

  useEffect(() => {
    if (pronta) void aggiornaCalendari();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pronta]);

  // ---- colori dinamici (config.colori_dinamici): un colore per azienda scelto sulla pagina che si guarda, al posto di quello fisso ----
  // Le tinte di prima restano da una lettura all'altra: cambia solo chi finisce accanto a una tinta uguale o vicina.
  const tintePrima = useRef<Map<string, string>>(new Map());
  const calendariAccesi = useMemo(() => {
    const spenti = new Set(Array.isArray(config.disabled_calendars) ? config.disabled_calendars : []);
    return calendariLetti.map((c) => ({ ...c, enabled: !spenti.has(c.id) }));
  }, [calendariLetti, config.disabled_calendars]);
  const colori = useMemo(() => {
    if (!config.colori_dinamici || inPubblico) return null;
    const aziende = calendariLetti.filter((c) => !c.builtin).map((c) => c.alias || c.account);
    return (tintePrima.current = coloriDinamici(eventiLetti, aziende, tintePrima.current, periodo ?? undefined));
  }, [config.colori_dinamici, inPubblico, calendariLetti, eventiLetti, periodo]);
  const calendari = useMemo(() => (colori ? calendariAccesi.map((c) => ({ ...c, color: colori.get(c.alias || c.account) ?? c.color })) : calendariAccesi), [calendariAccesi, colori]);
  const eventi = useMemo(() => (colori ? eventiLetti.map((e) => ({ ...e, color: colori.get(e.alias || e.account) ?? e.color })) : eventiLetti), [eventiLetti, colori]);

  const gruppi = useMemo(() => raggruppaAziende(calendari), [calendari]);

  // ---- eventi ----------------------------------------------------------------------------
  const leggi = useCallback(
    async (da: Date, a: Date): Promise<EventiRisposta> => {
      let dati: EventiRisposta;
      if (!inPubblico) dati = await api("get_events", isoLocale(da), isoLocale(a), null, spentiRef.current);
      else {
        const q = new URLSearchParams({ da: isoLocale(da), a: isoLocale(a), ...(gruppoPubblico ? { gruppo: gruppoPubblico } : {}) });
        const r = await fetch(`/api/disponibilita?${q}`);
        const j = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error((j && j.error) || `HTTP ${r.status}`);
        dati = j;
      }
      return dati.events ? { ...dati, events: dati.events.map(inOraDelDispositivo) } : dati;
    },
    [inPubblico, gruppoPubblico],
  );

  useEffect(() => {
    if (!periodo) return;
    const mia = ++richiesta.current;
    const scorre = aScorrimento(periodo.start, periodo.end);
    if (!scorre) cache.current = null;

    // Giorno, 3 giorni e settimana scorrono: ogni passo cambia periodo, e sotto il dito si vedono anche i giorni accanto. Per non
    // lasciare colonne vuote fino alla risposta si legge un intorno largo e si risponde dalla memoria finché il periodo ci sta dentro.
    if (scorre && dentroLaCache(cache.current, periodo.start, periodo.end)) {
      const c = cache.current!;
      if (vicinoAlBordo(c, periodo.start, periodo.end) && !rabboccoInCorso.current) {
        rabboccoInCorso.current = true;
        const largo = periodoLargo(periodo.start, periodo.end);
        leggi(largo.da, largo.a)
          .then((res) => {
            // cache cambiata nel frattempo (aggiornamento forzato, altra vista): la lettura è vecchia
            if (cache.current !== c || res.outlook_starting) return;
            cache.current = largo;
            setEventi(res.events || []);
            setStatoFonti(res.fonti || {});
          })
          .catch(() => {})
          .finally(() => {
            rabboccoInCorso.current = false;
          });
      }
      return;
    }

    const largo = scorre ? periodoLargo(periodo.start, periodo.end) : { da: periodo.start, a: periodo.end };
    mostraAvviso(null);
    syncInizio();
    leggi(largo.da, largo.a)
      .then((res) => {
        if (mia !== richiesta.current) {
          syncFine(false);
          return;
        }
        if (res.outlook_starting) {
          // Le caselle non rispondono ancora: si mostrano i dati salvati e si riprova da soli.
          eraFerma.current = true;
          setDatiVecchi(testoDatiVecchi(res.stale_at, res.avviso_sorgente));
          pianificaRitentativo();
          syncFine(false);
        } else {
          tentativi.current = 0;
          setDatiVecchi(null);
          if (res.errors?.length) mostraAvviso({ testo: res.errors.join("  ·  "), tipo: "errore" });
          syncFine(true);
          // le caselle sono tornate dopo un periodo di dati salvati: va riletto anche l'elenco
          if (eraFerma.current) {
            eraFerma.current = false;
            void aggiornaCalendariRef.current();
          }
        }
        setEventi(res.events || []);
        setStatoFonti(res.fonti || {});
        setPallinoInAttesa(false);
        cache.current = scorre && !res.outlook_starting ? largo : null;
      })
      .catch((err) => {
        syncFine(false);
        if (mia !== richiesta.current) return;
        mostraAvviso({ testo: "Eventi non caricati: " + (err instanceof Error ? err.message : String(err)), tipo: "errore" });
        // lettura fallita: i pallini non possono restare a pulsare all'infinito
        setStatoFonti({});
        setPallinoInAttesa(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [periodo, token]);

  // rilettura automatica: così un evento nuovo compare senza fare nulla
  useEffect(() => {
    const t = setInterval(() => {
      if (inCorsoRef.current === 0) ricarica();
    }, intervalloSync * 1000);
    return () => clearInterval(t);
  }, [intervalloSync, ricarica]);

  const forzaSync = useCallback(() => {
    setPallinoInAttesa(true);   // i pallini si spengono subito: niente verde della lettura precedente
    ricarica();
  }, [ricarica]);

  const secondari = useMemo(() => secondariEffettivi(pers.interfaccia.calendari_secondari, config.secondari_scelti), [pers, config.secondari_scelti]);
  const eventiGriglia = useMemo(() => eventiPerGriglia(eventi, { secondari, ricerca }), [eventi, secondari, ricerca]);
  const infoGiorni = useMemo(() => raccogliFestivi(eventi), [eventi]);

  // ---- impostazioni ----------------------------------------------------------------------
  // aspetto, vista e calendari accesi restano in questo dispositivo (VOCI_DEL_DISPOSITIVO), il resto va sul server
  const salvaConfig = useCallback(async (patch: Partial<Config>) => {
    setConfig((c) => ({ ...c, ...patch }));
    if (inPubblico) return;
    const mio = soloDelDispositivo(patch);
    if (Object.keys(mio).length) scriviDispositivo({ ...leggiDispositivo(), ...mio });
    const delServer = soloDelServer(patch);
    if (!Object.keys(delServer).length) return;
    try {
      await api("save_config", delServer);
    } catch {
      // la rete non c'è: l'impostazione vale in questa pagina, non resta salvata
    }
  }, [inPubblico]);

  // ---- calendari accesi: nel dispositivo, e get_events riceve quelli spenti --------------------------------------------
  const nomeDi = (c: Calendario) => c.alias || c.account;
  const scegliAccesi = useCallback(
    async (acceso: (c: Calendario) => boolean | undefined) => {
      const spenti = cambiaSpenti(calendariLetti, spentiRef.current, acceso);
      spentiRef.current = spenti;
      await salvaConfig({ disabled_calendars: spenti });
      ricarica();
    },
    [calendariLetti, salvaConfig, ricarica],
  );
  const accendiAzienda = useCallback((azienda: string, on: boolean) => scegliAccesi((c) => (nomeDi(c) === azienda ? on : undefined)), [scegliAccesi]);
  const accendiCalendario = useCallback((id: string, on: boolean) => scegliAccesi((c) => (c.id === id ? on : undefined)), [scegliAccesi]);
  /** Lascia accesa una sola azienda, o tutte se `azienda` è vuota. */
  const soloAzienda = useCallback((azienda: string) => scegliAccesi((c) => !azienda || nomeDi(c) === azienda), [scegliAccesi]);

  // ---- gruppi di calendari: stanno in config.gruppi_calendari, il filtro accende solo le loro aziende ---------
  const gruppiCalendari = useMemo(() => leggiGruppi(config.gruppi_calendari), [config.gruppi_calendari]);
  const gruppiAccesi = useMemo(() => gruppiAttivi(gruppiCalendari, gruppi), [gruppiCalendari, gruppi]);
  const salvaGruppi = useCallback((g: GruppiCalendari) => salvaConfig({ gruppi_calendari: g }), [salvaConfig]);
  const impostaSecondario = useCallback((nome: string, on: boolean) => salvaConfig({ secondari_scelti: { ...config.secondari_scelti, [nome]: on } }), [salvaConfig, config.secondari_scelti]);
  const soloGruppo = useCallback(
    async (nome: string) => {
      const membri = gruppiCalendari[nome];
      if (membri) await scegliAccesi((c) => membri.includes(nomeDi(c)));
    },
    [gruppiCalendari, scegliAccesi],
  );

  const cambiaZoom = useCallback(
    (verso: 1 | -1 | 0) => {
      const nuovo = verso === 0 ? ZOOM_DEFAULT : Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom + verso * ZOOM_PASSO));
      if (nuovo === zoom) return;
      setZoom(nuovo);
      void salvaConfig({ zoom_calendario: nuovo });
    },
    [zoom, salvaConfig],
  );

  const impostaIntervallo = useCallback(
    (secondi: number) => {
      setIntervallo(secondi);
      void salvaConfig({ reminder_poll_seconds: secondi });
    },
    [salvaConfig],
  );

  const tema: TemaConfig = theme === "light" || theme === "dark" ? theme : "auto";
  const impostaTema = useCallback(
    (t: TemaConfig) => {
      setTheme(t === "auto" ? "system" : t);
      void salvaConfig({ theme: t });
    },
    [setTheme, salvaConfig],
  );

  const valori: Valori = {
    pubblico: inPubblico, gruppoPubblico,
    pronta, pers, ricaricaPers, config, vistaIniziale,
    calendari, gruppi, statoFonti, pallinoInAttesa, aggiornaCalendari, accendiAzienda, accendiCalendario, soloAzienda,
    gruppiCalendari, gruppiAccesi, soloGruppo, salvaGruppi, secondari, impostaSecondario,
    periodo, impostaPeriodo, eventi, eventiGriglia, infoGiorni, ricerca, impostaRicerca, ricarica, forzaSync,
    avviso, datiVecchi, sincronizzando: inCorso > 0, etichetta: etichettaSync(ultimaSync, adesso, inCorso),
    salvaConfig, zoom, cambiaZoom, intervalloSync, impostaIntervallo,
    tema, temaScuro: resolvedTheme === "dark", impostaTema, mostraAvviso,
  };
  return <Contesto.Provider value={valori}>{children}</Contesto.Provider>;
}
