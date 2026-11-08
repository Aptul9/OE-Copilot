// Quanti lavori vanno insieme, e una coda corta per gli altri. Serve a chi apre un collegamento verso un sito che non si conosce (i loghi):
// senza un tetto, sessanta richieste insieme sono sessanta socket aperti verso chi risponde piano. I posti si prendono in ordine d'arrivo;
// chi trova la coda piena, o aspetta più del dovuto, riceve l'errore di chi ha costruito il semaforo. Nessun timer resta acceso: la scadenza
// si cancella quando il posto arriva.

type Voce = { passa: (rilascia: () => void) => void; rifiuta: (e: Error) => void; timer: ReturnType<typeof setTimeout> };

export class Semaforo {
  private occupati = 0;
  private readonly coda: Voce[] = [];

  constructor(
    readonly posti: number,
    readonly codaMax: number,
    readonly attesaMs: number,
    private readonly troppi: () => Error,
  ) {}

  get inUso(): number {
    return this.occupati;
  }

  get inCoda(): number {
    return this.coda.length;
  }

  /**
   * La funzione che rende il posto, una volta sola; il posto passa al primo della coda senza mai risultare libero, ma al giro dopo del
   * ciclo di eventi: chi ha appena finito (un socket chiuso) ha già lasciato il ciclo, in questo processo e in un server di prova, prima
   * che ne parta un altro, e i collegamenti aperti insieme restano quelli dei posti.
   */
  private rilascio(): () => void {
    let reso = false;
    return () => {
      if (reso) return;
      reso = true;
      const prossimo = this.coda.shift();
      if (!prossimo) {
        this.occupati--;
        return;
      }
      clearTimeout(prossimo.timer);
      setImmediate(() => prossimo.passa(this.rilascio()));
    };
  }

  /** Un posto subito, o null se sono tutti occupati: per chi può rimandare e non vuole stare in coda. */
  provaPrendere(): (() => void) | null {
    if (this.occupati >= this.posti) return null;
    this.occupati++;
    return this.rilascio();
  }

  /** Un posto: subito, o dopo la coda. Con la coda piena, o oltre `attesaMs`, solleva l'errore dato alla costruzione. */
  prendi(): Promise<() => void> {
    const subito = this.provaPrendere();
    if (subito) return Promise.resolve(subito);
    if (this.coda.length >= this.codaMax) return Promise.reject(this.troppi());
    return new Promise((passa, rifiuta) => {
      const voce: Voce = {
        passa,
        rifiuta,
        timer: setTimeout(() => {
          this.coda.splice(this.coda.indexOf(voce), 1);
          rifiuta(this.troppi());
        }, this.attesaMs),
      };
      this.coda.push(voce);
    });
  }

  /** Esegue il lavoro con un posto, e il posto si rende sempre, anche se il lavoro fallisce. */
  async esegui<T>(lavoro: () => Promise<T>): Promise<T> {
    const rilascia = await this.prendi();
    try {
      return await lavoro();
    } finally {
      rilascia();
    }
  }
}
