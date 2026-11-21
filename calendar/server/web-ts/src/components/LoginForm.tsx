"use client";

import { EyeIcon, EyeOffIcon, KeyRoundIcon, TriangleAlertIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { ServerApp } from "@/components/ServerApp";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field, FieldGroup, FieldLabel, FieldSeparator } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Spinner } from "@/components/ui/spinner";
import { authClient } from "@/lib/auth-client";
import type { Modo2fa } from "@/lib/config";

type Passo = "password" | "codice" | "recupero";

/** Il testo per un codice rifiutato. `ricomincia`: la sfida è chiusa (scaduta o troppi tentativi), si rientra dalla password. */
function erroreCodice(e: { status?: number; code?: string }, recupero: boolean): { testo: string; ricomincia: boolean } {
  if (e.code === "ACCOUNT_TEMPORARILY_LOCKED") return { testo: "Troppi tentativi, riprova tra 15 minuti", ricomincia: false };
  if (e.status === 429) return { testo: "Troppe richieste, riprova tra poco", ricomincia: false };
  if (e.code === "INVALID_TWO_FACTOR_COOKIE" || e.code === "TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE") return { testo: "Accesso scaduto, rientra con la password", ricomincia: true };
  return { testo: recupero ? "Codice di recupero errato" : "Codice errato", ricomincia: false };
}

export function LoginForm({ next, modo = "off", accessoEsterno = false }: { next: string; modo?: Modo2fa; accessoEsterno?: boolean }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [show, setShow] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [passo, setPasso] = useState<Passo>("password");
  const [codice, setCodice] = useState("");
  // dopo il montaggio: sul server il browser non si conosce, e le due pagine devono coincidere
  const [passkey, setPasskey] = useState(false);
  useEffect(() => setPasskey(modo === "passkey" && typeof window.PublicKeyCredential !== "undefined"), [modo]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    const { data, error } = await authClient.signIn.email({ email: email.trim(), password, rememberMe: true });
    setBusy(false);
    if (error) {
      setError(error.status === 429 ? error.message || "Troppi tentativi, riprova tra qualche minuto" : error.status === 403 ? "Accesso negato" : error.status === 503 ? "Accesso temporaneamente non disponibile" : "Credenziali non valide");
      return;
    }
    // password giusta ma manca il codice: il server non ha dato nessuna sessione
    if ((data as { twoFactorRedirect?: boolean } | null)?.twoFactorRedirect) {
      setCodice("");
      setPasso("codice");
      return;
    }
    // navigazione completa: la pagina e l'API stanno dietro al gateway, fuori dal router di Next
    window.location.href = next;
  }

  async function verificaCodice(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    const recupero = passo === "recupero";
    let pulito = codice.replace(/\s+/g, "");
    // i codici di recupero sono xxxxx-xxxxx: il trattino al centro si perdona
    if (recupero && /^[A-Za-z0-9]{10}$/.test(pulito)) pulito = `${pulito.slice(0, 5)}-${pulito.slice(5)}`;
    const { error } = recupero ? await authClient.twoFactor.verifyBackupCode({ code: pulito }) : await authClient.twoFactor.verifyTotp({ code: pulito });
    setBusy(false);
    if (error) {
      const { testo, ricomincia } = erroreCodice(error, recupero);
      setError(testo);
      if (ricomincia) {
        setPasso("password");
        setPassword("");
      }
      setCodice("");
      return;
    }
    window.location.href = next;
  }

  async function conPasskey() {
    setBusy(true);
    setError("");
    const { error } = await authClient.signIn.passkey();
    setBusy(false);
    if (error) {
      setError(error.status === 401 ? "Passkey non riconosciuta" : "Accesso con passkey non riuscito");
      return;
    }
    window.location.href = next;
  }

  const inCodice = passo !== "password";

  return (
    <div className="relative grid min-h-dvh place-items-center overflow-hidden bg-background px-4 py-10">
      <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 h-96 bg-[radial-gradient(ellipse_at_top,var(--color-accent),transparent_70%)]" />
      <Card className="relative w-full max-w-sm shadow-lg">
        <CardHeader className="justify-items-center gap-2 text-center">
          <img src="/icon.png" alt="" width={64} height={64} className="mb-2 size-16" />
          <CardTitle className="text-xl">Accedi a OE Copilot</CardTitle>
          {inCodice && <CardDescription>{passo === "codice" ? "Scrivi il codice dell'app di autenticazione." : "Scrivi uno dei codici di recupero."}</CardDescription>}
        </CardHeader>
        <CardContent>
          <form onSubmit={inCodice ? verificaCodice : submit}>
            <FieldGroup>
              {error && (
                <Alert variant="destructive">
                  <TriangleAlertIcon />
                  <AlertTitle>{error}</AlertTitle>
                </Alert>
              )}
              {!inCodice && (
                <>
                  <Field>
                    <FieldLabel htmlFor="email">{accessoEsterno ? "Email o nome utente" : "Email"}</FieldLabel>
                    <Input
                      id="email"
                      name="email"
                      type={accessoEsterno ? "text" : "email"}
                      autoCapitalize="none"
                      autoCorrect="off"
                      autoComplete="username"
                      className="h-10"
                      value={email}
                      onChange={(e) => setEmail(e.target.value)}
                      aria-invalid={!!error || undefined}
                      required
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="password">Password</FieldLabel>
                    <InputGroup className="h-10">
                      <InputGroupInput
                        id="password"
                        name="password"
                        type={show ? "text" : "password"}
                        autoComplete="current-password"
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        aria-invalid={!!error || undefined}
                        required
                      />
                      <InputGroupAddon align="inline-end">
                        <InputGroupButton size="icon-sm" onClick={() => setShow(!show)} aria-label={show ? "Nascondi la password" : "Mostra la password"} aria-pressed={show}>
                          {show ? <EyeOffIcon /> : <EyeIcon />}
                        </InputGroupButton>
                      </InputGroupAddon>
                    </InputGroup>
                  </Field>
                </>
              )}
              {inCodice && (
                <Field>
                  <FieldLabel htmlFor="codice">{passo === "recupero" ? "Codice di recupero" : "Codice a 6 cifre"}</FieldLabel>
                  <Input
                    id="codice"
                    name="codice"
                    inputMode={passo === "recupero" ? "text" : "numeric"}
                    autoComplete="one-time-code"
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                    maxLength={passo === "recupero" ? 11 : 7}
                    className="h-10 font-mono tracking-widest"
                    value={codice}
                    onChange={(e) => setCodice(e.target.value)}
                    aria-invalid={!!error || undefined}
                    required
                    autoFocus
                  />
                </Field>
              )}
              <Button type="submit" className="h-10 w-full" disabled={busy}>
                {busy && <Spinner />}
                {busy ? "Accesso in corso…" : inCodice ? "Verifica" : "Accedi"}
              </Button>
              {inCodice && (
                <div className="flex flex-col items-center gap-1">
                  <Button
                    type="button"
                    variant="ghost"
                    id="cambia-codice"
                    onClick={() => {
                      setError("");
                      setCodice("");
                      setPasso(passo === "codice" ? "recupero" : "codice");
                    }}
                  >
                    {passo === "codice" ? "Usa un codice di recupero" : "Usa il codice dell'app"}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    id="torna-password"
                    onClick={() => {
                      setError("");
                      setPasso("password");
                      setPassword("");
                    }}
                  >
                    Indietro
                  </Button>
                </div>
              )}
              {passkey && !inCodice && (
                <>
                  <FieldSeparator>oppure</FieldSeparator>
                  <Button type="button" variant="outline" id="accedi-passkey" className="h-10 w-full" disabled={busy} onClick={conPasskey}>
                    <KeyRoundIcon />
                    Accedi con una passkey
                  </Button>
                </>
              )}
            </FieldGroup>
          </form>
          <ServerApp className="mt-5 text-center text-sm text-muted-foreground" />
        </CardContent>
      </Card>
    </div>
  );
}
