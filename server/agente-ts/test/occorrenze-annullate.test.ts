import { describe, expect, it } from "vitest";
import { espandiRigheAndroid, type RigaAndroid } from "../src/android.ts";

// Quando un'occorrenza di una serie è annullata. Il telefono scrive STATUS_CANCELED (2) sulla riga che la sostituisce, ma Outlook lo scrive anche
// per le serie copiate da un flusso nel calendario di chi le legge (isOrganizer 1, nessun invitato): per tutte le occorrenze future, mentre le
// mostra e non le dà per annullate (verificato il 2026-10-13 sull'occorrenza delle 15:30 di una riunione di Tesco). Lì lo stato non conta.

const ms = (iso: string) => Date.parse(iso);
const BASE = {
  _sync_id: null,
  eventLocation: null,
  description: null,
  eventEndTimezone: null,
  allDay: 0,
  rdate: null,
  exrule: null,
  exdate: null,
  original_sync_id: null,
  originalAllDay: 0,
};
const serie = (extra: Partial<RigaAndroid> = {}): RigaAndroid => ({
  ...BASE,
  _id: 1,
  title: "Aggiornamento settimanale",
  eventStatus: null,
  dtstart: ms("2026-10-13T13:30:00Z"),
  dtend: null,
  duration: "P1800S",
  eventTimezone: "Europe/Rome",
  rrule: "FREQ=WEEKLY;UNTIL=20261230T000000Z;BYDAY=TU",
  original_id: null,
  originalInstanceTime: null,
  ...extra,
});
const eccezione = (extra: Partial<RigaAndroid> = {}): RigaAndroid => ({
  ...BASE,
  _id: 2,
  title: "Aggiornamento settimanale",
  eventStatus: 2,
  dtstart: ms("2026-10-13T13:30:00Z"),
  dtend: ms("2026-10-13T14:00:00Z"),
  duration: null,
  eventTimezone: "Europe/Rome",
  rrule: null,
  original_id: 1,
  originalInstanceTime: ms("2026-10-13T13:30:00Z"),
  ...extra,
});
const inizi = (righe: RigaAndroid[]) => espandiRigheAndroid(righe, "2026-10-12T00:00:00", "2026-10-28T00:00:00").map((o) => o.inizio);
const fini = (righe: RigaAndroid[]) => espandiRigheAndroid(righe, "2026-10-12T00:00:00", "2026-10-28T00:00:00").map((o) => o.fine);

const TUTTE = ["2026-10-13T15:30:00", "2026-10-20T15:30:00", "2026-10-27T15:30:00"];
const SENZA_LA_PRIMA = TUTTE.slice(1);

describe("occorrenza che sostituisce una della serie, con lo stato annullato", () => {
  it("senza nessuna eccezione la serie esce intera", () => expect(inizi([serie()])).toEqual(TUTTE));

  it("una copia di chi la legge, senza invitati (isOrganizer 1, hasAttendeeData 0): l'occorrenza c'è", () => {
    expect(inizi([serie(), eccezione({ isOrganizer: 1, hasAttendeeData: 0 })])).toEqual(TUTTE);
  });

  it("un annullamento con invitati (isOrganizer 0, hasAttendeeData 1): l'occorrenza non c'è", () => {
    expect(inizi([serie(), eccezione({ isOrganizer: 0, hasAttendeeData: 1 })])).toEqual(SENZA_LA_PRIMA);
  });

  it("organizzatore ma con invitati (hasAttendeeData 1): è un annullamento vero, non c'è", () => {
    expect(inizi([serie(), eccezione({ isOrganizer: 1, hasAttendeeData: 1 })])).toEqual(SENZA_LA_PRIMA);
  });

  it("senza le colonne (database vecchi, prove) o con valori vuoti: non c'è, come prima", () => {
    expect(inizi([serie(), eccezione()])).toEqual(SENZA_LA_PRIMA);
    expect(inizi([serie(), eccezione({ isOrganizer: null, hasAttendeeData: null })])).toEqual(SENZA_LA_PRIMA);
  });

  it("l'eccezione con lo stato normale resta una modifica dell'occorrenza, orario compreso", () => {
    const spostata = eccezione({ eventStatus: 1, isOrganizer: 1, hasAttendeeData: 0, dtstart: ms("2026-10-13T15:00:00Z"), dtend: ms("2026-10-13T15:30:00Z") });
    expect(inizi([serie(), spostata])).toEqual(["2026-10-13T17:00:00", ...SENZA_LA_PRIMA]);
  });

  it("una riga sola (niente serie) con lo stato annullato non esce, anche se è di chi la legge", () => {
    const sola: RigaAndroid = { ...serie(), rrule: null, eventStatus: 2, isOrganizer: 1, hasAttendeeData: 0, dtend: ms("2026-10-13T14:00:00Z"), duration: null };
    expect(inizi([sola])).toEqual([]);
  });

  it("la copia con lo stato annullato di una sola occorrenza non fa uscire le altre", () => {
    const seconda = eccezione({ _id: 3, originalInstanceTime: ms("2026-10-20T13:30:00Z"), dtstart: ms("2026-10-20T13:30:00Z"), dtend: ms("2026-10-20T14:00:00Z"), isOrganizer: 0, hasAttendeeData: 1 });
    expect(inizi([serie(), eccezione({ isOrganizer: 1, hasAttendeeData: 0 }), seconda])).toEqual(["2026-10-13T15:30:00", "2026-10-27T15:30:00"]);
  });
});

describe("la fine di una copia con lo stato annullato", () => {
  const copia = (extra: Partial<RigaAndroid> = {}) => eccezione({ isOrganizer: 1, hasAttendeeData: 0, ...extra });
  it("uguale all'inizio (come la scrive Outlook): vale la durata della serie, 30 minuti", () => {
    const zero = copia({ dtend: ms("2026-10-13T13:30:00Z") });
    expect(fini([serie(), zero])).toEqual(["2026-10-13T16:00:00", "2026-10-20T16:00:00", "2026-10-27T16:00:00"]);
  });
  it("mancante del tutto: vale la durata della serie", () => {
    expect(fini([serie(), copia({ dtend: null, duration: null })])[0]).toBe("2026-10-13T16:00:00");
  });
  it("la serie con la fine scritta e non la durata: si misura la sua", () => {
    const m = serie({ dtend: ms("2026-10-13T14:30:00Z"), duration: null });
    expect(fini([m, copia({ dtend: ms("2026-10-13T13:30:00Z") })])[0]).toBe("2026-10-13T16:30:00");
  });
  it("una fine vera della copia resta, 45 minuti", () => {
    expect(fini([serie(), copia({ dtend: ms("2026-10-13T14:15:00Z") })])[0]).toBe("2026-10-13T16:15:00");
  });
  it("la serie senza durata: la copia resta com'è", () => {
    const m = serie({ duration: null, dtend: null });
    expect(fini([m, copia({ dtend: ms("2026-10-13T13:30:00Z") })])[0]).toBe("2026-10-13T15:30:00");
  });
});
