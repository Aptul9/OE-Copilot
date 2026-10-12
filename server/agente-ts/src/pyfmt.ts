// Come Python scrive, confronta e converte un valore: str(), repr(), bool(), strip(), json.dumps(ensure_ascii=False) e l'ordine delle
// stringhe. L'agente Python risponde con queste forme e il gateway le legge così: qui sono riprodotte una per una.

/** I caratteri che str.strip() e \s di re considerano spazio: non sono quelli di String.trim() (U+001C a U+001F e U+0085 sì, U+FEFF no). */
export const SPAZI_PY = "\\t\\n\\v\\f\\r\\x1c-\\x1f \\x85\\xa0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000";
const SPAZI_AI_BORDI = new RegExp(`^[${SPAZI_PY}]+|[${SPAZI_PY}]+$`, "g");

export const pyStrip = (s: string): string => s.replace(SPAZI_AI_BORDI, "");

/** Il nome del tipo di un valore JSON come lo scrive Python nei suoi errori ("'list' object has no attribute 'get'"). */
export function pyTipo(v: unknown): string {
  if (v === null || v === undefined) return "NoneType";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "number") return Number.isInteger(v) ? "int" : "float";
  if (typeof v === "string") return "str";
  return Array.isArray(v) ? "list" : "dict";
}

/** bool() di un valore JSON. */
export function pyBool(v: unknown): boolean {
  if (v === null || v === undefined || v === false || v === 0 || v === "") return false;
  if (Array.isArray(v)) return v.length > 0;
  if (typeof v === "object") return Object.keys(v as object).length > 0;
  return true;
}

const NON_STAMPABILE = /[\p{Cc}\p{Cf}\p{Cs}\p{Co}\p{Cn}\p{Zl}\p{Zp}\p{Zs}]/u;
const hex = (n: number, cifre: number) => n.toString(16).padStart(cifre, "0");

/** repr() di una str: apici singoli, doppi se il testo ha un apice singolo e nessuno doppio, escape dei caratteri non stampabili. */
export function pyReprStr(s: string): string {
  const q = s.includes("'") && !s.includes('"') ? '"' : "'";
  let out = q;
  for (const ch of s) {
    const c = ch.codePointAt(0)!;
    if (ch === q || ch === "\\") out += "\\" + ch;
    else if (ch === "\n") out += "\\n";
    else if (ch === "\r") out += "\\r";
    else if (ch === "\t") out += "\\t";
    else if (c < 0x20 || c === 0x7f) out += "\\x" + hex(c, 2);
    else if (c < 0x7f) out += ch;
    else if (NON_STAMPABILE.test(ch)) out += c <= 0xff ? "\\x" + hex(c, 2) : c <= 0xffff ? "\\u" + hex(c, 4) : "\\U" + hex(c, 8);
    else out += ch;
  }
  return out + q;
}

/** repr() di un valore JSON (liste, dizionari, None, True, False, numeri, stringhe). */
export function pyRepr(v: unknown): string {
  if (v === null || v === undefined) return "None";
  if (typeof v === "boolean") return v ? "True" : "False";
  if (typeof v === "number") return String(v);
  if (typeof v === "string") return pyReprStr(v);
  if (Array.isArray(v)) return "[" + v.map(pyRepr).join(", ") + "]";
  return "{" + Object.entries(v as object).map(([k, x]) => pyReprStr(k) + ": " + pyRepr(x)).join(", ") + "}";
}

/** repr() di un timedelta di giorni, secondi e microsecondi già normalizzati (Python 3.12). */
export function pyReprDelta(giorni: number, secondi: number, micro: number): string {
  const parti: string[] = [];
  if (giorni) parti.push(`days=${giorni}`);
  if (secondi) parti.push(`seconds=${secondi}`);
  if (micro) parti.push(`microseconds=${micro}`);
  return `datetime.timedelta(${parti.length ? parti.join(", ") : "0"})`;
}

/** Il confronto fra stringhe di Python: per punto di codice, non per unità UTF-16 (un'emoji viene dopo U+FF21, non prima). */
export function confrontaPy(a: string, b: string): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    const x = a.charCodeAt(i);
    const y = b.charCodeAt(i);
    if (x === y) continue;
    const ordine = (c: number) => (c >= 0xe000 ? c - 0x800 : c >= 0xd800 ? c + 0x2000 : c);
    return ordine(x) < ordine(y) ? -1 : 1;
  }
  return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
}

/** json.dumps(dati, ensure_ascii=False): separatori ", " e ": ", chiavi nell'ordine in cui sono state inserite (una Map vale come un dizionario). */
export function jsonPy(v: unknown): string {
  if (v === null || v === undefined) return "null";
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") return Number.isNaN(v) ? "NaN" : v === Infinity ? "Infinity" : v === -Infinity ? "-Infinity" : String(v);
  if (typeof v === "string") return JSON.stringify(v);
  if (Array.isArray(v)) return "[" + v.map(jsonPy).join(", ") + "]";
  const voci = v instanceof Map ? [...v.entries()] : Object.entries(v as object);
  return "{" + voci.map(([k, x]) => JSON.stringify(k) + ": " + jsonPy(x)).join(", ") + "}";
}
