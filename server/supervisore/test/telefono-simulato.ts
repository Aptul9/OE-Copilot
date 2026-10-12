// Un telefono inventato che risponde ai comandi del ciclo con lo stato di un giro: lo usa la prova del ciclo (ciclo.test.ts). Gli scenari
// sono quelli con cui lo script bash del vecchio servizio systemd registrò le sequenze di comandi in test/golden/ciclo-<scenario>.txt.

export interface Fase {
  vivo: boolean;
  /** Schermo acceso all'inizio del giro. */
  sveglio: boolean;
  /** Dove torna Outlook quando lo si apre dall'icona: sulla vista principale o su un'altra schermata (le impostazioni). */
  ripresa: "principale" | "altra";
  bloccato: boolean;
  lavori: string;
}

export interface Scenario {
  nome: string;
  giri: number;
  accessiOgni: number;
  googleOgni: number;
  toccoOgni: number;
  /** Una fase per giro: l'ultima vale anche per i giri dopo. */
  fasi: Fase[];
  /** Per giro: c'è telefono/PAUSA. */
  pausaManuale: boolean[];
  /** Per giro: null = stato/schermo/PAUSA non c'è, altrimenti i secondi di età all'inizio del giro. */
  pausaSchermo: (number | null)[];
}

export interface StatoTelefono {
  sveglio: boolean;
  fuoco: "nessuna" | "iniziale" | "outlook" | "impostazioni";
  fase: Fase;
}

export const INIZIO = 1_760_000_000;
export const OGNI = 30;

export const LAVORI_DEL_TELEFONO = [
  "  JOB #1000/9: 7a1b2c3 com.android.calendar/com.microsoft.office.outlook.sync.CalendarSyncService",
  "  JOB #1000/12: 1f2e3d4 com.android.calendar/com.google.android.syncadapters.calendar.CalendarSyncAdapterService",
  "  JOB #1000/10: 9d8e7f6 com.android.calendar/com.microsoft.office.outlook.sync.CalendarSyncService",
  "  JOB #1000/3: ab12cd3 com.android.contacts/com.google.android.syncadapters.contacts.ContactsSyncAdapterService",
  "  JOB #1000/9: 7a1b2c3 com.android.calendar/com.microsoft.office.outlook.sync.CalendarSyncService",
  "  JOB #10080/4: 5e6f7a8 com.microsoft.office.outlook/androidx.work.impl.background.systemjob.SystemJobService",
  "",
].join("\n");

export const faseNormale = (): Fase => ({ vivo: true, sveglio: true, ripresa: "principale", bloccato: false, lavori: LAVORI_DEL_TELEFONO });

export function nuovoStato(fase: Fase): StatoTelefono {
  return { sveglio: fase.sveglio, fuoco: "nessuna", fase };
}

/** All'inizio di un giro il telefono torna com'è la fase; lo schermo e la finestra in primo piano non si ricordano dal giro prima. */
export function iniziaGiro(stato: StatoTelefono, fase: Fase): void {
  stato.fase = fase;
  stato.sveglio = fase.sveglio;
}

const FINESTRE = {
  nessuna: "null",
  iniziale: "Window{4d5e6f u0 com.android.launcher3/com.android.launcher3.uioverrides.QuickstepLauncher}",
  outlook: "Window{1a2b3c u0 com.microsoft.office.outlook/com.acompli.acompli.CentralActivity}",
  impostazioni: "Window{7c8d9e u0 com.microsoft.office.outlook/com.acompli.acompli.SettingsActivity}",
} as const;

const OUTLOOK = "com.microsoft.office.outlook/.MainActivity";

export interface Risposta {
  uscita: string;
  riuscito: boolean;
}

/** Il comando come lo riceve `docker exec <telefono> ...` o `adb shell ...`: l'elenco degli argomenti. */
export function rispondi(stato: StatoTelefono, comando: readonly string[]): Risposta {
  const testo = comando.join(" ");
  const f = stato.fase;
  const ok = (uscita = ""): Risposta => ({ uscita, riuscito: true });
  if (testo === "true") return { uscita: "", riuscito: f.vivo };
  if (!f.vivo) return { uscita: "", riuscito: false };
  if (testo === "dumpsys jobscheduler") return ok(f.lavori);
  if (testo === "dumpsys power") {
    return ok(`Power Manager State:\n  mWakefulness=${stato.sveglio ? "Awake" : "Asleep"}\n  mWakefulnessChanging=false\n  mHoldingWakeLockSuspendBlocker=false\n`);
  }
  if (testo === "dumpsys window") {
    return ok(
      `WINDOW MANAGER LAST ANR (dumpsys window lastanr)\n  mLastFocus=Window{0a0b0c u0 com.android.systemui/com.android.systemui.StatusBar}\n` +
        `  mCurrentFocus=${FINESTRE[stato.fuoco]}\n  mFocusedApp=ActivityRecord{abcdef u0 com.microsoft.office.outlook/.MainActivity t12}\n` +
        `  mCurrentFocus=Window{000000 u0 SecondaDisplay}\n  KeyguardController:\n    isKeyguardShowing=${f.bloccato}\n`,
    );
  }
  if (testo === "input keyevent 224") {
    stato.sveglio = true;
    return ok();
  }
  if (testo === "am start -a android.intent.action.MAIN -c android.intent.category.HOME") {
    stato.fuoco = "iniziale";
    return ok("Starting: Intent { act=android.intent.action.MAIN cat=[android.intent.category.HOME] }\n");
  }
  if (testo === `am start -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -n ${OUTLOOK}`) {
    stato.fuoco = f.ripresa === "principale" ? "outlook" : "impostazioni";
    return ok();
  }
  if (testo === `am start -f 0x10008000 -n ${OUTLOOK}`) {
    stato.fuoco = "outlook";
    return ok();
  }
  if (/^cmd jobscheduler run -f android \d+$/.test(testo) || testo === "input tap 360 5") return ok();
  return { uscita: `comando non previsto dal telefono inventato: ${testo}\n`, riuscito: false };
}

const scenario = (nome: string, resto: Partial<Scenario> & { fasi: Fase[] }): Scenario => ({
  nome,
  giri: 8,
  accessiOgni: 2,
  googleOgni: 4,
  toccoOgni: 3,
  pausaManuale: [false],
  pausaSchermo: [null],
  ...resto,
});

const normale = faseNormale();
const addormentato: Fase = { ...normale, sveglio: false };
const nelleImpostazioni: Fase = { ...normale, ripresa: "altra" };
const bloccata: Fase = { ...normale, bloccato: true };
const spento: Fase = { ...normale, vivo: false };

/** Gli scenari di confronto fra lo script e il ciclo in TypeScript: ognuno tocca un ramo diverso delle decisioni dello script. */
export const SCENARI: Scenario[] = [
  scenario("normale", { fasi: [normale] }),
  scenario("schermo-spento", { fasi: [addormentato, normale, addormentato, addormentato] }),
  scenario("outlook-nelle-impostazioni", { fasi: [nelleImpostazioni, normale, nelleImpostazioni] }),
  scenario("telefono-bloccato", { fasi: [bloccata, normale, bloccata] }),
  scenario("telefono-spento", { fasi: [normale, spento, spento, normale, spento] }),
  scenario("pausa-manuale", { fasi: [addormentato, nelleImpostazioni, normale], pausaManuale: [true, true, false, true, false, false, true, false] }),
  scenario("pausa-dello-schermo", {
    fasi: [addormentato, nelleImpostazioni, normale],
    pausaSchermo: [5, 59, 60, 61, null, 0, 3600, 20],
  }),
  scenario("tocco-ogni-giro", { toccoOgni: 1, fasi: [normale, addormentato, normale] }),
  scenario("tocco-spento", { toccoOgni: 0, fasi: [normale] }),
  scenario("google-ogni-giro", { googleOgni: 1, accessiOgni: 1, fasi: [normale] }),
  scenario("giri-lunghi", { giri: 25, toccoOgni: 10, googleOgni: 4, accessiOgni: 2, fasi: [normale, normale, addormentato, nelleImpostazioni, bloccata, spento, normale] }),
];
