"use client";

import { useSyncExternalStore } from "react";
import { LoginForm } from "@/components/LoginForm";
import { AppCalendario } from "@/components/oe/AppCalendario";
import { SicurezzaForm } from "@/components/SicurezzaForm";
import type { Modo2fa } from "@/lib/config";

// Le pagine dell'interfaccia esportata (OE_ESPORTA=1, app/**/*.esporta.tsx) per il gateway in Rust: il controllo della sessione lo fa il
// gateway, e i valori che la pagina riceveva dal server arrivano in window.__OE__, scritto nella testata prima di ogni script.

const nessunCambio = () => () => {};

/**
 * I valori del gateway. Nel disegno fatto alla costruzione (e nel primo disegno nel browser) valgono i predefiniti, così l'HTML ha già la
 * pagina e non un foglio bianco finché non arriva il JavaScript.
 */
function useValoriServer<T>(predefiniti: T): T {
  return useSyncExternalStore(
    nessunCambio,
    () => ((window as { __OE__?: T }).__OE__ ?? predefiniti),
    () => predefiniti,
  );
}

const HOME = { accesso2fa: "off" as Modo2fa };
const LOGIN = { next: "/", modo: "off" as Modo2fa };
const SICUREZZA = { modo: "totp" as Exclude<Modo2fa, "off">, email: "", attivo: false };
const DISPONIBILITA = { gruppo: "" };

export function HomeEsportata() {
  const v = useValoriServer(HOME);
  return <AppCalendario accesso2fa={v.accesso2fa} />;
}

export function LoginEsportata() {
  const v = useValoriServer(LOGIN);
  return <LoginForm next={v.next} modo={v.modo} />;
}

// key: SicurezzaForm tiene `attivo` nel suo stato, e la pagina pubblica legge per gruppo; con i valori veri si ricomincia da capo
export function SicurezzaEsportata() {
  const v = useValoriServer(SICUREZZA);
  return <SicurezzaForm key={`${v.modo}|${v.email}|${v.attivo}`} modo={v.modo} email={v.email} attivo={v.attivo} />;
}

export function DisponibilitaEsportata() {
  const v = useValoriServer(DISPONIBILITA);
  return <AppCalendario key={v.gruppo} pubblico={{ gruppo: v.gruppo }} />;
}
