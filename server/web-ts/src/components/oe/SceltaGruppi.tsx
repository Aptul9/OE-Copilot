"use client";

import { useOe } from "./OeProvider";

/** I gruppi con cui parte "A chi": quello che si sta guardando, se la vista è quella di un gruppo solo; altrimenti nessuno. */
export function gruppiDiPartenza(accesi: string[]): string[] {
  return accesi.length === 1 ? [...accesi] : [];
}

/** "A chi" di un evento personale: una casella per gruppo dei calendari (Gestisci calendari), anche più d'una. L'evento si vede nella vista
 * di ciascun gruppo scelto e conta nella sua disponibilità; in quella di tutti esce una volta. Senza gruppi non c'è niente da scegliere. */
export function SceltaGruppi({ id, scelti, onCambia }: { id: string; scelti: string[]; onCambia: (g: string[]) => void }) {
  const nomi = Object.keys(useOe().gruppiCalendari);
  if (!nomi.length) return null;
  const cambia = (nome: string, on: boolean) => onCambia(on ? [...scelti.filter((g) => g !== nome), nome] : scelti.filter((g) => g !== nome));
  return (
    <fieldset id={id} className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-[13px]">
      <legend className="float-left mr-1 text-xs font-medium">A chi</legend>
      {nomi.map((nome) => (
        <label key={nome} className="flex items-center gap-2">
          <input type="checkbox" className="size-4 accent-primary" data-gruppo={nome} checked={scelti.includes(nome)} onChange={(e) => cambia(nome, e.target.checked)} />
          {nome}
        </label>
      ))}
    </fieldset>
  );
}
