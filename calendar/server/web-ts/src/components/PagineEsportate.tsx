"use client";

import { useSyncExternalStore } from "react";
import { LoginForm } from "@/components/LoginForm";
import { AppCalendario } from "@/components/oe/AppCalendario";
import { SicurezzaForm } from "@/components/SicurezzaForm";
import type { Modo2fa } from "@/lib/config";

// Le pagine dell'interfaccia esportata (OE_ESPORTA=1, app/**/*.esporta.tsx) per il gateway in Rust: il controllo della sessione lo fa il
// gateway, e i valori che la pagina riceveva dal server arrivano in window.__OE__, scritto nella testata prima di ogni script.

const nessunCambio = () => () => {};

/** I valori del gateway: null nel disegno fatto alla costruzione, e finché la pagina non è nel browser. */
function useValoriServer<T>(): T | null {
  return useSyncExternalStore(
    nessunCambio,
    () => ((window as { __OE__?: T }).__OE__ ?? null),
    () => null,
  );
}

export function HomeEsportata() {
  const v = useValoriServer<{ accesso2fa: Modo2fa }>();
  return v && <AppCalendario accesso2fa={v.accesso2fa} />;
}

export function LoginEsportata() {
  const v = useValoriServer<{ next: string; modo: Modo2fa }>();
  return v && <LoginForm next={v.next} modo={v.modo} />;
}

export function SicurezzaEsportata() {
  const v = useValoriServer<{ modo: Exclude<Modo2fa, "off">; email: string; attivo: boolean }>();
  return v && <SicurezzaForm modo={v.modo} email={v.email} attivo={v.attivo} />;
}

export function DisponibilitaEsportata() {
  const v = useValoriServer<{ gruppo: string }>();
  return v && <AppCalendario pubblico={{ gruppo: v.gruppo }} />;
}
