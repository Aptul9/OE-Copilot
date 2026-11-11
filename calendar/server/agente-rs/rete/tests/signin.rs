//! Le prove di `agente-ts/test/signin.test.ts`.

use oe_nucleo::json::Json;
use oe_rete::prova_util::CartellaTemp;
use oe_rete::signin::{SignIns, campo};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

fn adesso() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

fn j(t: &str) -> Json {
    Json::analizza(t).unwrap()
}

struct Archivio {
    _dir: CartellaTemp,
    file: PathBuf,
    ora: Rc<Cell<i64>>,
}

impl Archivio {
    fn nuovo() -> Self {
        let d = CartellaTemp::nuova();
        let file = d.percorso().join("signin.json");
        Archivio { _dir: d, file, ora: Rc::new(Cell::new(1000)) }
    }

    fn con_ora(&self) -> SignIns {
        let ora = self.ora.clone();
        SignIns::nuovo(&self.file, move || ora.get())
    }

    fn vero(&self) -> SignIns {
        SignIns::nuovo(&self.file, adesso)
    }
}

#[test]
fn un_lavoro_per_indirizzo_che_tiene_l_id_a_ogni_avvio() {
    let a = Archivio::nuovo();
    let primo = a.vero().start(" USER@EXAMPLE.COM ", &j(r#"["mail","chat"]"#)).unwrap();
    assert_eq!(a.vero().start("user@example.com", &j(r#"["mail","chat"]"#)).unwrap()["id"], primo["id"]);
    let l = a.vero().list().unwrap();
    assert_eq!((campo(&l[0], "account"), l[0]["capabilities"].in_js(), campo(&l[0], "phase")), ("user@example.com", r#"["mail","chat"]"#.to_string(), "queued"));
}

#[test]
fn rifiuta_capacita_indirizzi_e_risultati_non_correlati() {
    let a = Archivio::nuovo();
    let jobs = a.vero();
    assert!(jobs.start("../evil", &j(r#"["mail"]"#)).is_err());
    assert!(jobs.start("a@b", &j(r#"["admin"]"#)).is_err());
    let lavoro = jobs.start("a@b", &j(r#"["calendar"]"#)).unwrap();
    assert!(jobs.find(&lavoro["id"], &Json::from("other@b")).is_err());
    assert!(jobs.update("missing", &[("phase", Some(Json::from("complete")))]).is_err());
}

#[test]
fn scade_senza_riprovare_per_sempre() {
    let a = Archivio::nuovo();
    let jobs = a.con_ora();
    jobs.start("a@b", &j(r#"["chat"]"#)).unwrap();
    a.ora.set(a.ora.get() + 1801);
    assert_eq!(campo(&jobs.list().unwrap()[0], "phase"), "blocked");
}

#[test]
fn dopo_cinque_minuti_si_ferma_e_la_fine_e_quel_momento() {
    let a = Archivio::nuovo();
    let jobs = a.con_ora();
    jobs.start("a@b", &j(r#"["mail","chat"]"#)).unwrap();
    a.ora.set(1300);
    assert_eq!(campo(&jobs.list().unwrap()[0], "phase"), "queued");
    a.ora.set(1301);
    let l = jobs.list().unwrap();
    assert_eq!(campo(&l[0], "phase"), "blocked");
    assert!(campo(&l[0], "error").contains("timed out"));
    assert_eq!(l[0]["ended"], Json::from(1300));
}

#[test]
fn la_fine_si_scrive_una_volta() {
    let a = Archivio::nuovo();
    let jobs = a.con_ora();
    let lavoro = jobs.start("a@b", &j(r#"["mail"]"#)).unwrap();
    a.ora.set(1050);
    jobs.update(campo(&lavoro, "id"), &[("phase", Some(Json::from("blocked"))), ("error", Some(Json::from("refused")))]).unwrap();
    a.ora.set(1100);
    jobs.update(campo(&lavoro, "id"), &[("phase", Some(Json::from("blocked"))), ("error", Some(Json::from("refused again")))]).unwrap();
    let l = jobs.list().unwrap();
    assert_eq!((campo(&l[0], "phase"), l[0]["ended"].clone()), ("blocked", Json::from(1050)));
}

#[test]
fn annulla_un_accesso_in_corso_o_fermo_mai_uno_finito() {
    let a = Archivio::nuovo();
    let jobs = a.con_ora();
    let in_corso = jobs.start("a@b", &j(r#"["mail","chat"]"#)).unwrap();
    a.ora.set(1020);
    let c = jobs.cancel(&in_corso["id"], &Json::from("a@b")).unwrap();
    assert_eq!((campo(&c, "phase"), campo(&c, "error"), c["ended"].clone()), ("blocked", "Connection cancelled.", Json::from(1020)));
    let tardi = jobs.start("c@d", &j(r#"["mail"]"#)).unwrap();
    a.ora.set(1400);
    // scaduto a 1320: annullare lo nasconde soltanto, la fine resta
    let c = jobs.cancel(&tardi["id"], &Json::from("c@d")).unwrap();
    assert_eq!((campo(&c, "error"), c["ended"].clone()), ("Connection cancelled.", Json::from(1320)));
    let finito = jobs.start("e@f", &j(r#"["mail"]"#)).unwrap();
    jobs.update(campo(&finito, "id"), &[("phase", Some(Json::from("complete")))]).unwrap();
    assert!(jobs.cancel(&finito["id"], &Json::from("e@f")).is_err());
    assert!(jobs.cancel(&in_corso["id"], &Json::from("other@b")).is_err());
}

#[test]
fn riprende_lo_stesso_browser_dopo_un_accesso_scaduto() {
    let a = Archivio::nuovo();
    let jobs = a.vero();
    let primo = jobs.start("a@b", &j(r#"["mail"]"#)).unwrap();
    jobs.update(campo(&primo, "id"), &[("slot", Some(Json::from(4))), ("phase", Some(Json::from("blocked"))), ("url", Some(Json::from("https://teams.example/api/desktop/4")))]).unwrap();
    let dopo = jobs.start("a@b", &j(r#"["mail"]"#)).unwrap();
    assert_eq!((dopo["slot"].clone(), campo(&dopo, "phase")), (Json::from(4), "queued"));
}
