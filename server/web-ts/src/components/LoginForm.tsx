"use client";

import { EyeIcon, EyeOffIcon, TriangleAlertIcon } from "lucide-react";
import { useState } from "react";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Spinner } from "@/components/ui/spinner";
import { authClient } from "@/lib/auth-client";

export function LoginForm({ next }: { next: string }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [show, setShow] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    const { error } = await authClient.signIn.email({ email: email.trim(), password, rememberMe: true });
    setBusy(false);
    if (error) {
      setError(error.status === 429 ? error.message || "Troppi tentativi, riprova fra qualche minuto" : "Email o password errate");
      return;
    }
    // navigazione completa: la pagina e l'API stanno dietro al gateway, fuori dal router di Next
    window.location.href = next;
  }

  return (
    <div className="relative grid min-h-dvh place-items-center overflow-hidden bg-background px-4 py-10">
      <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 h-96 bg-[radial-gradient(ellipse_at_top,var(--color-accent),transparent_70%)]" />
      <Card className="relative w-full max-w-sm shadow-lg">
        <CardHeader className="justify-items-center gap-2 text-center">
          <div className="mb-2 grid size-12 place-items-center rounded-2xl bg-primary text-lg font-bold text-primary-foreground">OE</div>
          <CardTitle className="text-xl">Accedi a OE Copilot</CardTitle>
          <CardDescription>Il calendario unico delle tue caselle.</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={submit}>
            <FieldGroup>
              {error && (
                <Alert variant="destructive">
                  <TriangleAlertIcon />
                  <AlertTitle>{error}</AlertTitle>
                </Alert>
              )}
              <Field>
                <FieldLabel htmlFor="email">Email</FieldLabel>
                <Input
                  id="email"
                  name="email"
                  type="email"
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
              <Button type="submit" className="h-10 w-full" disabled={busy}>
                {busy && <Spinner />}
                {busy ? "Accesso in corso…" : "Accedi"}
              </Button>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
