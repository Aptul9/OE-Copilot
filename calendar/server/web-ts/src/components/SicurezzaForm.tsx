"use client";

import { ArrowLeftIcon, CheckIcon, CopyIcon, KeyRoundIcon, ShieldCheckIcon, TriangleAlertIcon, Trash2Icon } from "lucide-react";
import { useEffect, useState } from "react";
import { CodiceQr } from "@/components/CodiceQr";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { authClient } from "@/lib/auth-client";
import type { Modo2fa } from "@/lib/config";

type Errore = { status?: number; code?: string } | null | undefined;

/** Il testo di un errore del server o del browser, in italiano: i messaggi del plugin sono in inglese e non si mostrano. */
function testoErrore(e: NonNullable<Errore>): string {
  if (e.code === "INVALID_PASSWORD") return "Password errata";
  if (e.code === "INVALID_CODE") return "Codice errato";
  if (e.code === "TOTP_ALREADY_ENABLED") return "Codice a due fattori già attivo";
  if (e.code === "SESSION_NOT_FRESH") return "Esci, rientra e riprova";
  if (e.code === "ERROR_AUTHENTICATOR_PREVIOUSLY_REGISTERED") return "Questo dispositivo ha già una passkey";
  if (e.code === "ERROR_CEREMONY_ABORTED" || e.code === "AUTH_CANCELLED") return "Operazione annullata";
  if (e.code === "ACCOUNT_TEMPORARILY_LOCKED") return "Troppi tentativi, riprova tra 15 minuti";
  if (e.status === 429) return "Troppe richieste, riprova tra poco";
  if (e.status === 401) return "Sessione scaduta, rientra con la password";
  return "Operazione non riuscita";
}

type Azione = "attiva" | "nuoviCodici" | "disattiva";
type Fase =
  | { tipo: "riposo" }
  | { tipo: "password"; azione: Azione }
  | { tipo: "scansione"; uri: string; codici: string[] }
  | { tipo: "codici"; codici: string[]; titolo: string };

const segretoDaUri = (uri: string) => new URL(uri).searchParams.get("secret") ?? "";
const spazia = (s: string) => s.replace(/(.{4})/g, "$1 ").trim();

export function SicurezzaForm({ modo, email, attivo: attivoIniziale }: { modo: Exclude<Modo2fa, "off">; email: string; attivo: boolean }) {
  return (
    <div className="min-h-dvh bg-background px-4 py-10">
      <div className="mx-auto flex w-full max-w-xl flex-col gap-6">
        <a href="/" id="sicurezza-indietro" className="inline-flex w-fit items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
          <ArrowLeftIcon className="size-4" />
          Torna al calendario
        </a>
        <div>
          <h1 className="text-xl font-semibold">Accesso e sicurezza</h1>
          <p className="mt-1 text-sm text-muted-foreground">Account: {email}</p>
        </div>
        <SezioneCodici attivoIniziale={attivoIniziale} />
        {modo === "passkey" && <SezionePasskey />}
      </div>
    </div>
  );
}

function SezioneCodici({ attivoIniziale }: { attivoIniziale: boolean }) {
  const [attivo, setAttivo] = useState(attivoIniziale);
  const [fase, setFase] = useState<Fase>({ tipo: "riposo" });
  const [password, setPassword] = useState("");
  const [codice, setCodice] = useState("");
  const [errore, setErrore] = useState("");
  const [occupato, setOccupato] = useState(false);
  const [copiato, setCopiato] = useState(false);

  const riposo = () => {
    setFase({ tipo: "riposo" });
    setPassword("");
    setCodice("");
    setErrore("");
    setCopiato(false);
  };
  const apri = (azione: Azione) => {
    setErrore("");
    setPassword("");
    setFase({ tipo: "password", azione });
  };

  async function confermaPassword(e: React.FormEvent) {
    e.preventDefault();
    if (fase.tipo !== "password") return;
    setOccupato(true);
    setErrore("");
    try {
      if (fase.azione === "attiva") {
        const { data, error } = await authClient.twoFactor.enable({ password, method: "totp" });
        if (error || data?.method !== "totp") return setErrore(testoErrore(error ?? {}));
        setFase({ tipo: "scansione", uri: data.totpURI, codici: data.backupCodes });
        setPassword("");
      } else if (fase.azione === "nuoviCodici") {
        const { data, error } = await authClient.twoFactor.generateBackupCodes({ password });
        if (error || !data) return setErrore(testoErrore(error ?? {}));
        setFase({ tipo: "codici", codici: data.backupCodes, titolo: "Nuovi codici di recupero" });
        setPassword("");
      } else {
        const { error } = await authClient.twoFactor.disable({ password });
        if (error) return setErrore(testoErrore(error));
        setAttivo(false);
        riposo();
      }
    } finally {
      setOccupato(false);
    }
  }

  async function confermaCodice(e: React.FormEvent) {
    e.preventDefault();
    if (fase.tipo !== "scansione") return;
    setOccupato(true);
    setErrore("");
    const { error } = await authClient.twoFactor.verifyTotp({ code: codice.replace(/\s+/g, "") });
    setOccupato(false);
    if (error) return setErrore(testoErrore(error));
    setAttivo(true);
    setCodice("");
    setFase({ tipo: "codici", codici: fase.codici, titolo: "Codici di recupero" });
  }

  async function copia(codici: string[]) {
    try {
      await navigator.clipboard.writeText(codici.join("\n"));
      setCopiato(true);
    } catch {
      setErrore("Copia non riuscita: copiali a mano");
    }
  }

  return (
    <Card id="sicurezza-codici">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheckIcon className="size-5" />
          Codice a due fattori
          <span id="sicurezza-stato" className={attivo ? "rounded-full bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-700 dark:text-emerald-400" : "rounded-full bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground"}>
            {attivo ? "Attivo" : "Non attivo"}
          </span>
        </CardTitle>
        <CardDescription>Dopo la password, un codice da un&apos;app di autenticazione.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {errore && (
          <Alert variant="destructive" id="sicurezza-errore">
            <TriangleAlertIcon />
            <AlertTitle>{errore}</AlertTitle>
          </Alert>
        )}

        {fase.tipo === "riposo" && (
          <div className="flex flex-wrap gap-2">
            {attivo ? (
              <>
                <Button id="sicurezza-nuovi-codici" variant="outline" onClick={() => apri("nuoviCodici")}>
                  Nuovi codici di recupero
                </Button>
                <Button id="sicurezza-disattiva" variant="outline" onClick={() => apri("disattiva")}>
                  Disattiva
                </Button>
              </>
            ) : (
              <Button id="sicurezza-attiva" onClick={() => apri("attiva")}>
                Attiva
              </Button>
            )}
          </div>
        )}

        {fase.tipo === "password" && (
          <form onSubmit={confermaPassword}>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="sicurezza-password">
                  {fase.azione === "disattiva" ? "Password per disattivare" : fase.azione === "nuoviCodici" ? "Password per i nuovi codici" : "Password per attivare"}
                </FieldLabel>
                <Input id="sicurezza-password" name="password" type="password" autoComplete="current-password" className="h-10" value={password} onChange={(e) => setPassword(e.target.value)} required autoFocus />
              </Field>
              <div className="flex gap-2">
                <Button id="sicurezza-password-ok" type="submit" disabled={occupato}>
                  {occupato && <Spinner />}
                  Continua
                </Button>
                <Button type="button" variant="ghost" onClick={riposo}>
                  Annulla
                </Button>
              </div>
            </FieldGroup>
          </form>
        )}

        {fase.tipo === "scansione" && (
          <form onSubmit={confermaCodice}>
            <FieldGroup>
              <div className="flex flex-col items-center gap-3 sm:flex-row sm:items-start">
                <CodiceQr testo={fase.uri} titolo="Codice QR" />
                <div className="flex min-w-0 flex-col gap-2 text-sm">
                  <p>1. Inquadra il codice con l&apos;app.</p>
                  <p className="text-muted-foreground">Oppure scrivi questa chiave (basata sul tempo):</p>
                  <code id="sicurezza-segreto" className="select-all break-all rounded-md bg-muted px-2 py-1.5 font-mono text-xs">
                    {spazia(segretoDaUri(fase.uri))}
                  </code>
                  <p>2. Scrivi il codice dell&apos;app.</p>
                </div>
              </div>
              <Field>
                <FieldLabel htmlFor="sicurezza-codice">Codice a 6 cifre</FieldLabel>
                <Input id="sicurezza-codice" name="codice" inputMode="numeric" autoComplete="one-time-code" pattern="[0-9 ]*" maxLength={7} className="h-10 w-40 font-mono tracking-widest" value={codice} onChange={(e) => setCodice(e.target.value)} required autoFocus />
              </Field>
              <div className="flex gap-2">
                <Button id="sicurezza-codice-ok" type="submit" disabled={occupato}>
                  {occupato && <Spinner />}
                  Conferma e attiva
                </Button>
                <Button type="button" variant="ghost" onClick={riposo}>
                  Annulla
                </Button>
              </div>
            </FieldGroup>
          </form>
        )}

        {fase.tipo === "codici" && (
          <div className="flex flex-col gap-3">
            <p className="text-sm font-medium">{fase.titolo}</p>
            <Alert>
              <TriangleAlertIcon />
              <AlertTitle>Salvali ora: non si vedranno più.</AlertTitle>
            </Alert>
            <ul id="sicurezza-codici-recupero" className="grid grid-cols-2 gap-x-4 gap-y-1 rounded-md bg-muted p-3 font-mono text-sm">
              {fase.codici.map((c) => (
                <li key={c} className="select-all">
                  {c}
                </li>
              ))}
            </ul>
            <div className="flex gap-2">
              <Button id="sicurezza-copia" type="button" variant="outline" onClick={() => copia(fase.codici)}>
                {copiato ? <CheckIcon /> : <CopyIcon />}
                {copiato ? "Copiati" : "Copia"}
              </Button>
              <Button id="sicurezza-codici-ok" type="button" onClick={riposo}>
                Li ho salvati
              </Button>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

type Passkey = { id: string; name?: string | null; createdAt?: string | Date | null; deviceType?: string | null };

function SezionePasskey() {
  const { data, isPending, error: erroreLista } = authClient.useListPasskeys();
  const [nome, setNome] = useState("");
  const [errore, setErrore] = useState("");
  const [occupato, setOccupato] = useState(false);
  // dopo il montaggio: sul server e al primo disegno il browser non si conosce, e le due pagine devono coincidere
  const [supportata, setSupportata] = useState(true);
  useEffect(() => setSupportata(typeof window.PublicKeyCredential !== "undefined"), []);
  const lista = (data ?? []) as Passkey[];

  async function aggiungi(e: React.FormEvent) {
    e.preventDefault();
    setOccupato(true);
    setErrore("");
    const { error } = await authClient.passkey.addPasskey({ name: nome.trim() || undefined });
    setOccupato(false);
    if (error) return setErrore(testoErrore(error));
    setNome("");
  }

  async function togli(id: string) {
    setErrore("");
    const { error } = await authClient.passkey.deletePasskey({ id });
    if (error) setErrore(testoErrore(error));
  }

  return (
    <Card id="sicurezza-passkey">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <KeyRoundIcon className="size-5" />
          Passkey
        </CardTitle>
        <CardDescription>Accesso senza password, con impronta, volto o PIN del dispositivo.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {errore && (
          <Alert variant="destructive" id="sicurezza-passkey-errore">
            <TriangleAlertIcon />
            <AlertTitle>{errore}</AlertTitle>
          </Alert>
        )}
        {erroreLista && <p className="text-sm text-muted-foreground">Elenco delle passkey non disponibile.</p>}
        <ul id="sicurezza-passkey-elenco" className="flex flex-col divide-y rounded-md border">
          {isPending && <li className="p-3 text-sm text-muted-foreground">Carico…</li>}
          {!isPending && lista.length === 0 && <li className="p-3 text-sm text-muted-foreground">Nessuna passkey.</li>}
          {lista.map((p) => (
            <li key={p.id} className="flex items-center justify-between gap-3 p-3 text-sm" data-passkey={p.id}>
              <div className="min-w-0">
                <p className="truncate font-medium">{p.name || "Passkey senza nome"}</p>
                <p className="text-xs text-muted-foreground">
                  {p.createdAt ? `Aggiunta il ${new Date(p.createdAt).toLocaleDateString("it-IT")}` : ""}
                  {p.deviceType === "multiDevice" ? " · sincronizzata" : p.deviceType === "singleDevice" ? " · solo su questo dispositivo" : ""}
                </p>
              </div>
              <Button variant="ghost" size="icon" aria-label={`Elimina la passkey ${p.name ?? ""}`} className="elimina-passkey" onClick={() => togli(p.id)}>
                <Trash2Icon />
              </Button>
            </li>
          ))}
        </ul>
        {supportata ? (
          <form onSubmit={aggiungi}>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="sicurezza-passkey-nome">Nome (facoltativo)</FieldLabel>
                <Input id="sicurezza-passkey-nome" name="nome" placeholder="es. Portatile" className="h-10" value={nome} onChange={(e) => setNome(e.target.value)} maxLength={60} />
              </Field>
              <div>
                <Button id="sicurezza-passkey-aggiungi" type="submit" disabled={occupato}>
                  {occupato ? <Spinner /> : <KeyRoundIcon />}
                  Aggiungi una passkey
                </Button>
              </div>
            </FieldGroup>
          </form>
        ) : (
          <p className="text-sm text-muted-foreground">Passkey non disponibili: serve un browser compatibile e HTTPS.</p>
        )}
      </CardContent>
    </Card>
  );
}
