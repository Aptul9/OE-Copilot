//! Quanti lavori vanno insieme, e una coda corta per gli altri (era lib/core/semaforo.ts). Serve a chi apre un collegamento verso un sito che
//! non si conosce: senza un tetto, sessanta richieste insieme sono sessanta socket aperti verso chi risponde piano. I posti si prendono in
//! ordine d'arrivo; chi trova la coda piena, o aspetta più del dovuto, riceve l'errore di chi ha costruito il semaforo.

use crate::py::{Errore, Esito};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;
use tokio::sync::oneshot;

struct Stato {
    occupati: usize,
    coda: VecDeque<(u64, oneshot::Sender<()>)>,
    prossimo: u64,
}

pub struct Semaforo {
    pub posti: usize,
    pub coda_max: usize,
    pub attesa: Duration,
    troppi: fn() -> Errore,
    stato: RefCell<Stato>,
}

/// Un posto preso: si rende quando si lascia cadere, una volta sola, anche se il lavoro è fallito.
pub struct Posto {
    semaforo: Rc<Semaforo>,
}

impl Drop for Posto {
    fn drop(&mut self) {
        self.semaforo.rilascia();
    }
}

impl Semaforo {
    pub fn new(posti: usize, coda_max: usize, attesa: Duration, troppi: fn() -> Errore) -> Rc<Self> {
        Rc::new(Semaforo { posti, coda_max, attesa, troppi, stato: RefCell::new(Stato { occupati: 0, coda: VecDeque::new(), prossimo: 0 }) })
    }

    pub fn in_uso(&self) -> usize {
        self.stato.borrow().occupati
    }

    pub fn in_coda(&self) -> usize {
        self.stato.borrow().coda.len()
    }

    /// Il posto passa al primo della coda senza mai risultare libero.
    fn rilascia(&self) {
        let mut s = self.stato.borrow_mut();
        while let Some((_, tx)) = s.coda.pop_front() {
            if tx.send(()).is_ok() {
                return;
            }
        }
        s.occupati -= 1;
    }

    /// Un posto subito, o None se sono tutti occupati: per chi può rimandare e non vuole stare in coda.
    pub fn prova_prendere(self: &Rc<Self>) -> Option<Posto> {
        let mut s = self.stato.borrow_mut();
        if s.occupati >= self.posti {
            return None;
        }
        s.occupati += 1;
        Some(Posto { semaforo: self.clone() })
    }

    /// Un posto: subito, o dopo la coda. Con la coda piena, o oltre `attesa`, l'errore dato alla costruzione.
    pub async fn prendi(self: &Rc<Self>) -> Esito<Posto> {
        if let Some(p) = self.prova_prendere() {
            return Ok(p);
        }
        let (tx, mut rx) = oneshot::channel();
        let id = {
            let mut s = self.stato.borrow_mut();
            if s.coda.len() >= self.coda_max {
                return Err((self.troppi)());
            }
            let id = s.prossimo;
            s.prossimo += 1;
            s.coda.push_back((id, tx));
            id
        };
        match tokio::time::timeout(self.attesa, &mut rx).await {
            Ok(Ok(())) => Ok(Posto { semaforo: self.clone() }),
            _ => {
                if rx.try_recv().is_ok() {
                    return Ok(Posto { semaforo: self.clone() });
                }
                self.stato.borrow_mut().coda.retain(|(x, _)| *x != id);
                Err((self.troppi)())
            }
        }
    }
}
