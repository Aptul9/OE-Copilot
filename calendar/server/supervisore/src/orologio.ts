// L'ora e le attese in un punto solo, così le prove le possono comandare.

export interface Orologio {
  /** Millisecondi dall'epoca. */
  adesso(): number;
  /** Aspetta `ms` millisecondi; se il segnale scatta prima, torna subito senza errore. */
  dormi(ms: number, segnale?: AbortSignal): Promise<void>;
}

export const orologioVero: Orologio = {
  adesso: () => Date.now(),
  dormi: (ms, segnale) =>
    new Promise<void>((risolvi) => {
      if (segnale?.aborted) return risolvi();
      const fine = () => {
        clearTimeout(timer);
        segnale?.removeEventListener("abort", fine);
        risolvi();
      };
      const timer = setTimeout(fine, ms);
      segnale?.addEventListener("abort", fine, { once: true });
    }),
};

/** Un orologio che non scorre da solo: `dormi` avanza l'ora di quanto gli si chiede e ricorda l'attesa. */
export class OrologioFinto implements Orologio {
  attese: number[] = [];
  suAttesa?: (ms: number) => void | Promise<void>;

  constructor(private ora: number) {}

  adesso = (): number => this.ora;

  async dormi(ms: number): Promise<void> {
    this.ora += ms;
    this.attese.push(ms);
    await this.suAttesa?.(ms);
  }

  /** Porta l'ora avanti senza contare un'attesa (per il tempo che passa mentre un comando lavora). */
  avanza(ms: number): void {
    this.ora += ms;
  }
}
