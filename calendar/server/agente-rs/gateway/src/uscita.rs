//! I collegamenti che il gateway apre verso siti che non si conoscono: i loghi (loghi.rs) e i calendari da link (ics.rs). I posti sono gli
//! stessi per tutti e due, così nessuno dei due serve ad aprire socket senza limite (era lib/core/uscita.ts).

use crate::py::Errore;
use crate::rete_sicura::SoloProve;
use crate::semaforo::Semaforo;
use std::rc::Rc;
use std::time::Duration;

/// Quanti collegamenti verso siti esterni stanno aperti insieme, in tutto.
pub const POSTI_MAX: usize = 4;
/// Chi aspetta un posto: al più CODA_MAX richieste, al più ATTESA_CODA_MS ciascuna; oltre, "Troppe richieste".
pub const CODA_MAX: usize = 16;
pub const ATTESA_CODA_MS: u64 = 5000;
pub const TROPPE_RICHIESTE: &str = "Troppe richieste, riprova tra poco";

thread_local! {
    static POSTI: Rc<Semaforo> = Semaforo::new(POSTI_MAX, CODA_MAX, Duration::from_millis(ATTESA_CODA_MS), || Errore::valore(TROPPE_RICHIESTE));
}

pub fn posti() -> Rc<Semaforo> {
    POSTI.with(Rc::clone)
}

thread_local! {
    static PROVE: std::cell::RefCell<Option<SoloProve>> = const { std::cell::RefCell::new(None) };
}

/// Solo per le prove: permette a un server di prova su 127.0.0.1 di rispondere. Nel binario del gateway non c'è (funzione "prove" del
/// crate): senza, scarica_sicuro usa solo le regole vere.
#[cfg(any(test, feature = "prove"))]
pub fn imposta_rete_di_prova(p: Option<SoloProve>) {
    PROVE.with(|x| *x.borrow_mut() = p);
}

/// Quello che una prova ha messo con imposta_rete_di_prova: i soli punti del gateway che scaricano (loghi.rs, ics.rs) lo passano a scarica_sicuro.
pub fn rete_di_prova() -> Option<SoloProve> {
    PROVE.with(|x| x.borrow().clone())
}
