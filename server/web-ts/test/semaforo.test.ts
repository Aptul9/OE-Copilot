import { describe, expect, it, vi } from "vitest";
import { Semaforo } from "@/lib/core/semaforo";

// I posti per i collegamenti verso l'esterno: quanti insieme, in che ordine, che cosa succede a chi arriva con la coda piena o aspetta troppo.

const attesa = (ms: number) => new Promise<void>((ok) => setTimeout(ok, ms));
const troppi = () => new Error("troppe richieste");
/** Un lavoro che finisce quando si decide: `fine()`. */
const aMano = () => {
  let fine!: () => void;
  const promessa = new Promise<void>((ok) => (fine = ok));
  return { promessa, fine };
};

describe("Semaforo", () => {
  it("non tiene più lavori insieme dei posti, e li riempie tutti", async () => {
    const s = new Semaforo(3, 100, 5000, troppi);
    let insieme = 0;
    let massimo = 0;
    const lavoro = async () => {
      massimo = Math.max(massimo, ++insieme);
      await attesa(5);
      insieme--;
    };
    await Promise.all(Array.from({ length: 30 }, () => s.esegui(lavoro)));
    expect(massimo).toBe(3);
    expect(s.inUso).toBe(0);
    expect(s.inCoda).toBe(0);
  });

  it("i posti vanno in ordine d'arrivo", async () => {
    const s = new Semaforo(1, 100, 5000, troppi);
    const ordine: number[] = [];
    await Promise.all(
      Array.from({ length: 12 }, (_, i) =>
        s.esegui(async () => {
          ordine.push(i);
          await attesa(1);
        }),
      ),
    );
    expect(ordine).toEqual(Array.from({ length: 12 }, (_, i) => i));
  });

  it("con la coda piena la richiesta in più riceve l'errore subito, senza aspettare", async () => {
    const s = new Semaforo(2, 3, 5000, troppi);
    const fermi = Array.from({ length: 5 }, aMano);
    const ok = fermi.map((f) => s.esegui(() => f.promessa));
    expect(s.inUso).toBe(2);
    expect(s.inCoda).toBe(3);
    const t0 = performance.now();
    await expect(s.esegui(async () => "non parte")).rejects.toThrow("troppe richieste");
    await expect(s.prendi()).rejects.toThrow("troppe richieste");
    expect(performance.now() - t0).toBeLessThan(100);
    expect(s.inCoda).toBe(3);
    for (const f of fermi) f.fine();
    await Promise.all(ok);
    expect(s.inUso).toBe(0);
    expect(s.inCoda).toBe(0);
  });

  it("chi aspetta più di attesaMs riceve l'errore, esce dalla coda e il suo lavoro non parte mai", async () => {
    const s = new Semaforo(1, 5, 40, troppi);
    const occupa = aMano();
    const primo = s.esegui(() => occupa.promessa);
    let partito = false;
    const t0 = performance.now();
    const secondo = s.esegui(async () => {
      partito = true;
    });
    await expect(secondo).rejects.toThrow("troppe richieste");
    expect(performance.now() - t0).toBeGreaterThanOrEqual(35);
    expect(performance.now() - t0).toBeLessThan(1000);
    expect(s.inCoda).toBe(0);
    occupa.fine();
    await primo;
    await attesa(20);
    expect(partito).toBe(false);
    expect(s.inUso).toBe(0);
    // e dopo, il semaforo funziona come prima
    expect(await s.esegui(async () => 7)).toBe(7);
  });

  it("chi non aspetta troppo ottiene il posto, e la scadenza non scatta più (nessun timer resta acceso)", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const s = new Semaforo(1, 5, 1000, troppi);
      const occupa = aMano();
      const primo = s.esegui(() => occupa.promessa);
      const secondo = s.esegui(async () => "fatto");
      expect(vi.getTimerCount()).toBe(1);
      await vi.advanceTimersByTimeAsync(900);
      occupa.fine();
      await primo;
      expect(await secondo).toBe("fatto");
      expect(vi.getTimerCount()).toBe(0);
      await vi.advanceTimersByTimeAsync(5000);
      expect(s.inUso).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("il posto passa al primo in coda senza risultare libero: nessuno che arrivi in quel momento lo prende", async () => {
    const s = new Semaforo(1, 5, 5000, troppi);
    const rilascia = s.provaPrendere() as () => void;
    expect(rilascia).toBeTypeOf("function");
    const aspetta = s.prendi();
    expect(s.inCoda).toBe(1);
    rilascia();
    expect(s.inUso).toBe(1);
    expect(s.inCoda).toBe(0);
    expect(s.provaPrendere()).toBeNull();
    (await aspetta)();
    expect(s.inUso).toBe(0);
    expect(s.provaPrendere()).toBeTypeOf("function");
  });

  it("il posto passa al giro dopo del ciclo di eventi, non nello stesso: chi ha appena chiuso un socket lo ha già lasciato prima che ne parta un altro", async () => {
    const s = new Semaforo(1, 5, 5000, troppi);
    const rilascia = s.provaPrendere() as () => void;
    const ordine: string[] = [];
    const aspetta = s.prendi().then((r) => {
      ordine.push("il primo in coda parte");
      return r;
    });
    setImmediate(() => ordine.push("un altro giro dello stesso ciclo"));
    rilascia();
    ordine.push("posto reso");
    // il posto è in viaggio, non libero: nessuno lo prende nel frattempo
    expect(s.inUso).toBe(1);
    expect(s.provaPrendere()).toBeNull();
    (await aspetta)();
    expect(ordine).toEqual(["posto reso", "un altro giro dello stesso ciclo", "il primo in coda parte"]);
    expect(s.inUso).toBe(0);
  });

  it("provaPrendere non aspetta: null se i posti sono tutti occupati; il posto reso due volte si libera una volta sola", () => {
    const s = new Semaforo(2, 5, 5000, troppi);
    const a = s.provaPrendere() as () => void;
    const b = s.provaPrendere() as () => void;
    expect(s.provaPrendere()).toBeNull();
    expect(s.inUso).toBe(2);
    a();
    a();
    a();
    expect(s.inUso).toBe(1);
    expect(s.provaPrendere()).toBeTypeOf("function");
    expect(s.provaPrendere()).toBeNull();
    b();
  });

  it("il posto si rende anche se il lavoro fallisce", async () => {
    const s = new Semaforo(1, 5, 5000, troppi);
    await expect(
      s.esegui(async () => {
        throw new Error("rotto");
      }),
    ).rejects.toThrow("rotto");
    expect(s.inUso).toBe(0);
    await expect(
      s.esegui(() => {
        throw new Error("rotto subito");
      }),
    ).rejects.toThrow("rotto subito");
    expect(s.inUso).toBe(0);
    expect(await s.esegui(async () => "ancora")).toBe("ancora");
  });

  it("tante richieste a caso: mai oltre i posti, ognuna fatta o rifiutata, e alla fine tutto pulito", async () => {
    let seme = 12345;
    const caso = () => ((seme = (seme * 1664525 + 1013904223) >>> 0) / 4294967296);
    const s = new Semaforo(4, 10, 30, troppi);
    let insieme = 0;
    let massimo = 0;
    let fatti = 0;
    let rifiutati = 0;
    const lavori = Array.from({ length: 300 }, async (_, i) => {
      await attesa(Math.floor(caso() * 40));
      try {
        await s.esegui(async () => {
          massimo = Math.max(massimo, ++insieme);
          await attesa(Math.floor(caso() * 15));
          insieme--;
          if (i % 7 === 0) throw new Error("rotto");
        });
        fatti++;
      } catch (e) {
        if ((e as Error).message === "troppe richieste") rifiutati++;
        else fatti++;
      }
    });
    await Promise.all(lavori);
    expect(massimo).toBeLessThanOrEqual(4);
    expect(fatti + rifiutati).toBe(300);
    expect(fatti).toBeGreaterThan(0);
    expect(rifiutati).toBeGreaterThan(0);
    expect(s.inUso).toBe(0);
    expect(s.inCoda).toBe(0);
  });
});
