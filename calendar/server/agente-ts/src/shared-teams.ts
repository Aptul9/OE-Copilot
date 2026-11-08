// Lo stato degli account Teams che TeamsRelay manda all'agente ogni minuto (POST /api/auth/shared/teams), perché il calendario mostri
// accanto a ogni casella se Teams la usa e come sta, e gli account Teams che hanno solo il loro accesso del browser. Sta in memoria: un
// riavvio lo perde fino al giro dopo. Un rapporto più vecchio di STANTIO_S non si mostra.
//   {"accounts": [{"account", "slot", "state", "detail"}], "others": [{"email", "slot", "state", "detail"}]}

export const STANTIO_S = 600;
const MAX_VOCI = 64;
const EMAIL = /^[^\s@]{1,128}@[^\s@]{1,253}$/;
const STATO = /^[a-z][a-z-]{0,23}$/;

export interface StatoTeams {
  slot: number;
  state: string;
  detail: string;
}

export interface RapportoTeams {
  at: number;
  accounts: Map<string, StatoTeams>;
  others: (StatoTeams & { email: string })[];
}

export class ErroreRapportoTeams extends Error {}

const rifiuta = (motivo: string): never => {
  throw new ErroreRapportoTeams(motivo);
};

function voce(dato: unknown, campo: "account" | "email"): StatoTeams & { email: string } {
  if (dato === null || typeof dato !== "object" || Array.isArray(dato)) return rifiuta("ogni voce è un oggetto");
  const r = dato as Record<string, unknown>;
  const email = typeof r[campo] === "string" ? (r[campo] as string).trim().toLowerCase() : "";
  // un account Teams senza identità ancora letta (appena aggiunto) non ha indirizzo: va bene solo tra gli altri
  if (!(campo === "email" && email === "") && !EMAIL.test(email)) return rifiuta(`${campo} non valido`);
  if (!Number.isInteger(r.slot) || (r.slot as number) < 1 || (r.slot as number) > 9999) return rifiuta("slot non valido");
  if (typeof r.state !== "string" || !STATO.test(r.state)) return rifiuta("state non valido");
  const detail = typeof r.detail === "string" ? r.detail.replace(/[\x00-\x1f\x7f]/g, " ").slice(0, 200) : "";
  return { email, slot: r.slot as number, state: r.state, detail };
}

/** Il corpo del rapporto, controllato: elenchi brevi, indirizzi, slot, stati come parole. Nessun altro campo. */
export function leggiRapportoTeams(corpo: unknown, adessoS: number): RapportoTeams {
  if (corpo === null || typeof corpo !== "object" || Array.isArray(corpo)) return rifiuta('serve {"accounts": [...], "others": [...]}');
  const r = corpo as Record<string, unknown>;
  const estranei = Object.keys(r).filter((k) => k !== "accounts" && k !== "others");
  if (estranei.length) return rifiuta(`campi non ammessi: ${estranei.join(", ").slice(0, 80)}`);
  const elenco = (k: string) => {
    const v = r[k] ?? [];
    if (!Array.isArray(v) || v.length > MAX_VOCI) return rifiuta(`${k}: un elenco di al massimo ${MAX_VOCI} voci`);
    return v as unknown[];
  };
  const accounts = new Map<string, StatoTeams>();
  for (const v of elenco("accounts")) {
    const { email, ...stato } = voce(v, "account");
    accounts.set(email, stato);
  }
  return { at: adessoS, accounts, others: elenco("others").map((v) => voce(v, "email")) };
}

/** Il rapporto se è recente, altrimenti nessuno: un TeamsRelay fermo non lascia stati vecchi sulla pagina. */
export function rapportoValido(r: RapportoTeams | undefined, adessoS: number): RapportoTeams | undefined {
  return r && adessoS - r.at <= STANTIO_S ? r : undefined;
}
