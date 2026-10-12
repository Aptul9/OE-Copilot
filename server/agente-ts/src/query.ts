// Il percorso e la query di una richiesta come li legge l'agente Python: self.path di http.server (che riduce "//" a "/") e urlparse e
// parse_qs di urllib.parse 3.12 (il punto e virgola nell'ultimo tratto del percorso è un parametro, "+" nella query è uno spazio, un
// valore vuoto non conta, vale il primo valore di un nome ripetuto).

import { ErroreSenzaRisposta } from "./tempo.ts";

const SCHEMA_CARATTERI = /^[A-Za-z0-9+.-]+$/;
const USA_PARAMETRI = new Set(["", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu", "sip", "sips", "mms", "sftp", "tel"]);

/** urlparse(self.path).path e .query. Un indirizzo che urlsplit rifiuta (ValueError) fa morire il gestore, come in Python. */
export function analizzaTarget(target: string): { percorso: string; query: string } {
  let url = target.startsWith("//") ? "/" + target.replace(/^\/+/, "") : target;
  url = url.replace(/^[\x00-\x20]+/, "").replace(/[\t\r\n]/g, "");
  let schema = "";
  const i = url.indexOf(":");
  if (i > 0 && /^[A-Za-z]/.test(url) && SCHEMA_CARATTERI.test(url.slice(0, i))) {
    schema = url.slice(0, i).toLowerCase();
    url = url.slice(i + 1);
  }
  if (url.startsWith("//")) {
    let fine = url.length;
    for (const c of "/?#") {
      const k = url.indexOf(c, 2);
      if (k >= 0) fine = Math.min(fine, k);
    }
    const netloc = url.slice(2, fine);
    url = url.slice(fine);
    if ((netloc.includes("[") && !netloc.includes("]")) || (netloc.includes("]") && !netloc.includes("["))) throw new ErroreSenzaRisposta("ValueError: Invalid IPv6 URL");
  }
  const cancelletto = url.indexOf("#");
  if (cancelletto >= 0) url = url.slice(0, cancelletto);
  let query = "";
  const punto = url.indexOf("?");
  if (punto >= 0) {
    query = url.slice(punto + 1);
    url = url.slice(0, punto);
  }
  if (USA_PARAMETRI.has(schema) && url.includes(";")) {
    const k = url.includes("/") ? url.indexOf(";", url.lastIndexOf("/")) : url.indexOf(";");
    if (k >= 0) url = url.slice(0, k);
  }
  return { percorso: url, query };
}

const ESADECIMALI = /^[0-9A-Fa-f]{2}/;

/** unquote(): le sequenze %XX valide diventano byte, lette poi come UTF-8 (un byte non valido diventa U+FFFD); i caratteri non ASCII restano. */
function unquote(s: string): string {
  if (!s.includes("%")) return s;
  const pezzi = s.split(/([\x00-\x7f]+)/);
  const decoder = new TextDecoder("utf-8");
  return pezzi
    .map((pezzo, k) => {
      if (k % 2 === 0) return pezzo;
      const byte: number[] = [];
      const parti = pezzo.split("%");
      for (const c of parti[0]!) byte.push(c.charCodeAt(0));
      for (const p of parti.slice(1)) {
        if (ESADECIMALI.test(p)) {
          byte.push(parseInt(p.slice(0, 2), 16));
          for (const c of p.slice(2)) byte.push(c.charCodeAt(0));
        } else for (const c of "%" + p) byte.push(c.charCodeAt(0));
      }
      return decoder.decode(Uint8Array.from(byte));
    })
    .join("");
}

const unquotePlus = (s: string) => unquote(s.replaceAll("+", " "));

/** parse_qs(): nome -> valori, senza i valori vuoti. */
export function parseQs(qs: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const campo of qs.split("&")) {
    if (!campo) continue;
    const i = campo.indexOf("=");
    const valore = i < 0 ? "" : campo.slice(i + 1);
    if (!valore) continue;
    const nome = unquotePlus(i < 0 ? campo : campo.slice(0, i));
    const lista = out.get(nome);
    if (lista) lista.push(unquotePlus(valore));
    else out.set(nome, [unquotePlus(valore)]);
  }
  return out;
}
