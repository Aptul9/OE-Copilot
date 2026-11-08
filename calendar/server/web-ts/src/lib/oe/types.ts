import type { CondivisioneEsito, CondivisioniRisposta } from "@/lib/condivisione-comune";

// Le forme dei dati che l'API del gateway (lib/core/api.ts) restituisce alla pagina. Sono quelle della
// vecchia pagina (app.js, nella storia di git fino a 2d57a83^): qui si danno solo i tipi.

export type Stato = "ok" | "avviso" | "errore";
export type FonteStato = { stato: Stato; msg: string };

/** Un evento come lo manda get_events. */
export type Evento = {
  id: string;
  title: string;
  start: string;
  end: string;
  allDay: boolean;
  color: string;
  account: string;
  alias?: string;
  calendar?: string;
  location?: string;
  organizer?: string;
  organizer_email?: string;
  online_url?: string | null;
  web_link?: string | null;
  nascosto?: boolean;
  is_local?: boolean;
  is_google?: boolean;
  remoto?: boolean;
  uid?: string;
  inoltrata_da?: string;
  invitato_come?: string;
  /** Evento personale: "Blocca il tempo" (impegni.ts); manca se è stato fatto prima che ci fosse. */
  blocca?: boolean;
  /** Evento personale: i gruppi di "A chi" (locali.ts); manca se non è di un gruppo in particolare. */
  gruppi?: string[];
  /** "Mostra come: Occupato" in Outlook o Google. */
  occupato?: boolean;
};

export type EventiRisposta = {
  events: Evento[];
  errors: string[];
  fonti?: Record<string, FonteStato>;
  stale?: boolean;
  stale_at?: string | null;
  outlook_starting?: boolean;
  avviso_sorgente?: string | null;
};

/** Un calendario dell'elenco (get_calendars). */
export type Calendario = {
  id: string;
  account: string;
  alias?: string;
  label: string;
  color: string;
  enabled: boolean;
  count?: number | null;
  nota?: string;
  url?: string | null;
  is_ics?: boolean;
  is_local?: boolean;
  is_inoltro?: boolean;
  builtin?: boolean;
  stato?: Stato | string;
  stato_msg?: string;
};

export type CalendariRisposta = { calendars: Calendario[]; error: string | null };

export type TemaConfig = "auto" | "light" | "dark";

/** Gruppi di calendari: nome del gruppo -> aziende (i nomi della legenda). Un'azienda può stare in più gruppi. */
export type GruppiCalendari = Record<string, string[]>;

export type Config = {
  theme?: TemaConfig;
  zoom_calendario?: number;
  /** Colori scelti sulla pagina che si guarda (coloriDinamici in logic.ts) al posto di quello fisso dell'azienda. */
  colori_dinamici?: boolean;
  /** Toglie dalla griglia i giorni interi e gli eventi che durano un giorno o più (daTogliere in logic.ts). */
  nascondi_tutto_il_giorno?: boolean;
  vista?: string;
  gruppi_calendari?: GruppiCalendari;
  /** Calendari secondari scelti dalla finestra Calendari: nome -> true (secondario) o false (principale, anche se il file dice il contrario). */
  secondari_scelti?: Record<string, boolean>;
  reminder_poll_seconds?: number;
  reminder_minutes_before?: number[];
  disabled_calendars?: string[];
  /** Eventi delle caselle e dei link eliminati dalla pagina (elimina_evento): il server non li manda più, finché non si ripristinano. */
  eventi_eliminati?: EventoEliminato[];
  [chiave: string]: unknown;
};

/** `eliminato_il` manca nelle voci di prima del 2026-11-07. */
export type EventoEliminato = { chiave: string; title: string; start: string; end: string; allDay: boolean; alias: string; eliminato_il?: string };

export type SchedaTimesheet = {
  nome: string;
  colore?: string;
  logo?: string;
  url?: string;
  azione?: string;
  sotto?: string;
};

/** Un logo scelto dalla finestra Calendari (lib/core/loghi.ts): l'indirizzo da cui viene (se non è un file caricato) e quando è stato letto. */
export type VoceLogo = { url: string | null; tipo: "url" | "file"; aggiornato: number };

export type Personalizzazione = {
  interfaccia: {
    schede_timesheet?: SchedaTimesheet[];
    loghi?: Record<string, string>;
    /** I nomi i cui loghi, scritti come indirizzo in personalizzazione.json, il server sta ancora scaricando: la pagina li rilegge dopo qualche secondo. */
    loghi_in_corso?: string[];
    sigle?: Record<string, string>;
    calendari_secondari?: string[];
    calendari_senza_teams?: string[];
  };
  estensioni?: string[];
};

export type NuovoEvento = {
  title: string;
  start: string;
  end: string;
  allDay: boolean;
  location: string;
  notes: string;
  blocca: boolean;
  /** "A chi": i gruppi dei calendari per cui è l'evento. */
  gruppi?: string[];
};

export type Esito = { ok: boolean; error?: string };

export type ContenutoPromemoria = "completo" | "minimo";
/** Le impostazioni dei promemoria, una sola per sito, app e server (lib/core/promemoria.ts). */
export type ImpostazioniPromemoria = { minuti: number[]; contenuto: ContenutoPromemoria; suono: string; notifica_windows: boolean; comparsa: boolean };
export type PromemoriaRisposta = {
  promemoria: ImpostazioniPromemoria;
  /**
   * I canali che il server conosce e se può usarli (le chiavi stanno nell'ambiente del server, mai qui). Per le notifiche del browser anche:
   * se le chiavi VAPID ci sono, la chiave pubblica (serve alla pagina per iscriversi) e quanti browser sono iscritti.
   */
  canali: { nome: string; configurato: boolean; chiavi?: boolean; chiave_pubblica?: string | null; iscrizioni?: number }[];
  ultimo_errore: string | null;
  ultimo_errore_il: string | null;
};

export type CasellaAccesso = {
  id: string;
  nome: string;
  account: string;
  provider: "microsoft" | "google";
  configurato: boolean;
  /** Serve un nuovo accesso: nessun accesso salvato, o il fornitore non lo accetta più. */
  scaduto?: boolean;
  stato: string;
  msg: string;
  /** Le aziende le cui copie arrivano in questa casella. */
  inoltrate?: string[];
};

/** Quello che una casella Microsoft condivisa lascia usare: posta, calendario, chat Teams (agente, shared-provisioning.ts). */
export type CapacitaCondivisa = "mail" | "calendar" | "chat";
/** La condivisione di una casella: null se non è condivisa. L'origine dice con quale client Microsoft rinnova l'accesso. */
export type Condivisione = { capabilities: CapacitaCondivisa[]; origin: "mobile" | "outlook-spa" | "teams-spa" } | null;

/** I metodi dell'API che il gateway serve (apiTs in lib/core/api.ts): argomenti e risposta. */
export type Metodi = {
  get_config: [[], Config];
  save_config: [[Partial<Config>], Esito];
  get_personalizzazione: [[], Personalizzazione];
  get_calendars: [[boolean?], CalendariRisposta];
  get_events: [[string, string, (boolean | null)?, string[]?], EventiRisposta];
  set_calendar_enabled: [[string, boolean], Esito];
  set_account_enabled: [[string, boolean], Esito];
  set_azienda_enabled: [[string, boolean], Esito & { calendari?: number }];
  mostra_solo_azienda: [[string], Esito];
  mostra_solo_gruppo: [[string], Esito];
  set_account_alias: [[string, string], Esito];
  add_ics_calendar: [[string, string], Esito & { feed?: { name: string } }];
  remove_ics_calendar: [[string], Esito];
  add_local_event: [[NuovoEvento], Esito];
  delete_local_event: [[string], Esito];
  elimina_evento: [[Evento], Esito & { chiave?: string; eventi_eliminati?: EventoEliminato[] }];
  ripristina_evento: [[string], Esito & { eventi_eliminati?: EventoEliminato[] }];
  get_promemoria: [[], PromemoriaRisposta];
  set_promemoria: [[Partial<ImpostazioniPromemoria>], Esito & { promemoria?: ImpostazioniPromemoria }];
  prova_promemoria: [[string], Esito];
  iscrivi_push: [[PushSubscriptionJSON], Esito];
  disiscrivi_push: [[string], Esito];
  get_condivisioni: [[], CondivisioniRisposta];
  add_condivisione: [[Record<string, unknown>], CondivisioneEsito];
  update_condivisione: [[string, Record<string, unknown>], CondivisioneEsito];
  revoke_condivisione: [[string], CondivisioneEsito];
  delete_condivisione: [[string], CondivisioneEsito];
  rotate_condivisione: [[string], CondivisioneEsito];
  get_loghi: [[], { loghi: Record<string, VoceLogo> }];
  preview_logo: [[string], Esito & { anteprima?: string; tipo?: string; byte?: number }];
  set_logo: [[string, string], Esito & { logo?: VoceLogo }];
  upload_logo: [[string, string], Esito & { logo?: VoceLogo }];
  refresh_logo: [[string], Esito & { logo?: VoceLogo }];
  remove_logo: [[string], Esito];
  get_stato_caselle: [[], Esito & { caselle: CasellaAccesso[] }];
  rimuovi_casella: [[string], Esito & { account?: string }];
  get_condivisione_microsoft: [[], Esito & { accounts: { account: string; shared: Condivisione }[] }];
  set_condivisione_microsoft: [[string, CapacitaCondivisa[] | null], Esito & { account?: string; shared?: Condivisione }];
  inizia_accesso_microsoft: [
    [string?, ("microsoft" | "google")?, ("device" | "pkce")?],
    Esito & {
      provider?: "microsoft" | "google";
      modo?: "device" | "pkce";
      url?: string;
      state?: string;
      user_code?: string;
      intervallo_s?: number;
      url_pkce?: string;
    },
  ];
  controlla_accesso_casella: [[string], Esito & { stato?: "in_attesa" | "completato"; account?: string; provider?: string }];
  completa_accesso_casella: [[Record<string, unknown>], Esito & { account?: string; provider?: string }];
};

export type NomeMetodo = keyof Metodi;
