// Le forme dei dati che l'API Python (app.Api) restituisce alla pagina. Sono quelle del file
// web/app.js: il servizio ui le serve così come sono, qui si danno solo i tipi.

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
  vista?: string;
  gruppi_calendari?: GruppiCalendari;
  reminder_poll_seconds?: number;
  reminder_minutes_before?: number[];
  [chiave: string]: unknown;
};

export type SchedaTimesheet = {
  nome: string;
  colore?: string;
  logo?: string;
  url?: string;
  azione?: string;
  sotto?: string;
};

export type Personalizzazione = {
  interfaccia: {
    schede_timesheet?: SchedaTimesheet[];
    loghi?: Record<string, string>;
    sigle?: Record<string, string>;
    calendari_secondari?: string[];
    calendari_senza_teams?: string[];
  };
  estensioni?: string[];
  sorgente?: "remota" | "outlook";
};

export type NuovoEvento = {
  title: string;
  start: string;
  end: string;
  allDay: boolean;
  location: string;
  notes: string;
};

export type Esito = { ok: boolean; error?: string };

/** I metodi di Api che il servizio ui espone (METODI di server/ui/server.py): argomenti e risposta. */
export type Metodi = {
  get_config: [[], Config];
  save_config: [[Partial<Config>], Esito];
  get_personalizzazione: [[], Personalizzazione];
  get_calendars: [[boolean?], CalendariRisposta];
  get_events: [[string, string], EventiRisposta];
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
};

export type NomeMetodo = keyof Metodi;
