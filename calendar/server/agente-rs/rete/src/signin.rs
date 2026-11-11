//! Gli accessi guidati in corso (`<tokens.json>.signins.json`): una casella, le capacità chieste, la fase (queued, browser, complete,
//! blocked), lo slot del browser di TeamsRelay. Un accesso non finito in SIGNIN_TIMEOUT_S si ferma e il suo slot torna libero.

use crate::errori::{ErroreAccesso, EsitoAccesso};
use oe_nucleo::json::{Json, Mappa};
use std::path::{Path, PathBuf};

// a sign-in not finished in this time stops; its browser slot is given back
pub const SIGNIN_TIMEOUT_S: i64 = 300;
pub const CANCELLED: &str = "Connection cancelled.";

/// Un accesso com'è nel file: l'oggetto resta com'è, con i campi aggiunti dagli aggiornamenti nell'ordine in cui arrivano.
pub type SignIn = Mappa;

pub fn campo<'a>(j: &'a SignIn, k: &str) -> &'a str {
    j.get(k).and_then(Json::as_str).unwrap_or_default()
}

fn numero(j: &SignIn, k: &str) -> f64 {
    j.get(k).and_then(Json::as_f64).unwrap_or(f64::NAN)
}

/// `crypto.randomUUID()`.
pub fn uuid() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

fn altro(m: &str) -> ErroreAccesso {
    ErroreAccesso::Altro(m.to_string())
}

pub struct SignIns {
    file: PathBuf,
    now: Box<dyn Fn() -> i64>,
}

fn indirizzo_valido(a: &str) -> bool {
    let vietato = |c: char| c.is_whitespace() || c == '@' || c == '/' || c == '?' || c == '#';
    let Some((l, d)) = a.split_once('@') else { return false };
    let conta = |s: &str| s.encode_utf16().count();
    !l.chars().any(vietato) && !d.chars().any(vietato) && (1..=128).contains(&conta(l)) && (1..=253).contains(&conta(d))
}

impl SignIns {
    pub fn nuovo(file: &Path, now: impl Fn() -> i64 + 'static) -> Self {
        SignIns { file: file.to_path_buf(), now: Box::new(now) }
    }

    fn read(&self) -> EsitoAccesso<Vec<SignIn>> {
        if !self.file.exists() {
            return Ok(Vec::new());
        }
        let testo = crate::file::leggi_utf8(&self.file).map_err(|e| ErroreAccesso::Altro(e.to_string()))?;
        match Json::analizza(&testo).map_err(|e| ErroreAccesso::Altro(e.messaggio))? {
            Json::Arr(v) => Ok(v.into_iter().map(|j| if let Json::Obj(m) = j { m } else { Mappa::new() }).collect()),
            _ => Err(altro("this.read(...).map is not a function")),
        }
    }

    fn save(&self, jobs: &[SignIn]) -> EsitoAccesso<()> {
        let testo = Json::Arr(jobs.iter().cloned().map(Json::Obj).collect()).in_js();
        crate::file::scrivi_atomico(&self.file, &testo, Some(0o600)).map_err(|e| ErroreAccesso::Altro(e.to_string()))
    }

    pub fn list(&self) -> EsitoAccesso<Vec<SignIn>> {
        let now = (self.now)();
        Ok(self
            .read()?
            .into_iter()
            .map(|mut j| {
                let fase = campo(&j, "phase");
                if fase != "complete" && fase != "blocked" && now as f64 - numero(&j, "created") > SIGNIN_TIMEOUT_S as f64 {
                    let fine = numero(&j, "created") + SIGNIN_TIMEOUT_S as f64;
                    j.insert("phase", "blocked");
                    j.insert("error", "Sign-in timed out. Resume when ready.");
                    j.insert("ended", fine);
                }
                j
            })
            .collect())
    }

    pub fn start(&self, address: &str, capabilities: &Json) -> EsitoAccesso<SignIn> {
        let account = crate::js::js_trim(address).to_lowercase();
        if !indirizzo_valido(&account) {
            return Err(altro("Valid account address required"));
        }
        let caps: Vec<String> = match capabilities {
            Json::Arr(v) if !v.is_empty() && v.iter().all(|c| matches!(c.as_str(), Some("mail" | "calendar" | "chat"))) => {
                let mut out: Vec<String> = Vec::new();
                for c in v.iter().filter_map(Json::as_str) {
                    if !out.iter().any(|x| x == c) {
                        out.push(c.to_string());
                    }
                }
                out
            }
            _ => return Err(altro("Choose mail, calendar or Teams")),
        };
        let jobs = self.list()?;
        let previous = jobs.iter().find(|j| campo(j, "account") == account).cloned();
        if let Some(p) = &previous {
            if matches!(campo(p, "phase"), "queued" | "browser") {
                let mut a = caps.clone();
                a.sort();
                let mut b: Vec<String> = p.get("capabilities").and_then(Json::as_arr).map(|v| v.iter().map(|c| c.as_str().unwrap_or_default().to_string()).collect()).unwrap_or_default();
                b.sort();
                if a != b {
                    return Err(altro("A connection is already in progress. Finish it before changing services."));
                }
                return Ok(p.clone());
            }
        }
        let mut job = Mappa::new();
        job.insert("id", uuid());
        job.insert("account", account.as_str());
        job.insert("capabilities", caps);
        job.insert("phase", "queued");
        job.insert("created", (self.now)());
        if let Some(p) = previous.filter(|p| p.get("slot").is_some_and(|s| s.as_f64().is_some_and(|n| n != 0.0))) {
            job.insert("slot", p["slot"].clone());
            if let Some(u) = p.get("url") {
                job.insert("url", u.clone());
            }
        }
        let mut altri: Vec<SignIn> = jobs.into_iter().filter(|j| campo(j, "account") != account).collect();
        if altri.len() > 63 {
            altri.drain(..altri.len() - 63);
        }
        altri.push(job.clone());
        self.save(&altri)?;
        Ok(job)
    }

    pub fn find(&self, id: &Json, account: &Json) -> EsitoAccesso<SignIn> {
        let j = self.list()?.into_iter().find(|j| j.get("id") == Some(id) && j.get("account") == Some(account));
        match j {
            Some(j) if !matches!(campo(&j, "phase"), "complete" | "blocked") => Ok(j),
            _ => Err(altro("Sign-in request expired or mismatched")),
        }
    }

    /// Le voci della modifica con `None` tolgono il campo (Object.assign con undefined, che JSON.stringify non scrive).
    pub fn update(&self, id: &str, patch: &[(&str, Option<Json>)]) -> EsitoAccesso<SignIn> {
        let mut jobs = self.read()?;
        let Some(j) = jobs.iter_mut().find(|j| campo(j, "id") == id) else { return Err(altro("Sign-in request not found")) };
        let mut patch: Vec<(&str, Option<Json>)> = patch.to_vec();
        let blocca = patch.iter().any(|(k, v)| *k == "phase" && v.as_ref().and_then(Json::as_str) == Some("blocked"));
        let con_fine = patch.iter().any(|(k, v)| *k == "ended" && v.as_ref().is_some_and(|x| !matches!(x, Json::Null | Json::Bool(false)) && x.as_f64() != Some(0.0)));
        if blocca && campo(j, "phase") != "blocked" && !con_fine {
            patch.retain(|(k, _)| *k != "ended");
            patch.push(("ended", Some(Json::from((self.now)()))));
        }
        // un campo messo a undefined resta nell'oggetto che torna (jsonPy lo scrive null) e sparisce dal file (JSON.stringify lo salta)
        let mut indefiniti: Vec<&str> = Vec::new();
        for (k, v) in patch {
            match v {
                Some(v) => {
                    indefiniti.retain(|x| *x != k);
                    j.insert(k, v);
                }
                None => {
                    indefiniti.push(k);
                    j.insert(k, Json::Null);
                }
            }
        }
        let fatto = j.clone();
        for k in indefiniti {
            j.remove(k);
        }
        self.save(&jobs)?;
        Ok(fatto)
    }

    // The owner stops a sign-in, running or already stopped: it no longer shows, and its slot is given back. A finished one stays as it is.
    pub fn cancel(&self, id: &Json, account: &Json) -> EsitoAccesso<SignIn> {
        let Some(j) = self.list()?.into_iter().find(|j| j.get("id") == Some(id) && j.get("account") == Some(account)) else {
            return Err(altro("Sign-in request not found"));
        };
        if campo(&j, "phase") == "complete" {
            return Err(altro("Sign-in already complete"));
        }
        let ended = if campo(&j, "phase") == "blocked" { j.get("ended").filter(|e| !e.is_null()).cloned().unwrap_or_else(|| Json::from((self.now)())) } else { Json::from((self.now)()) };
        self.update(campo(&j, "id"), &[("phase", Some(Json::from("blocked"))), ("error", Some(Json::from(CANCELLED))), ("ended", Some(ended))])
    }
}

#[cfg(test)]
mod prove {
    use super::*;
    use crate::prova_util::CartellaTemp;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn avvio_scadenza_e_annullamento() {
        let d = CartellaTemp::nuova();
        let ora = Rc::new(Cell::new(1000i64));
        let o2 = ora.clone();
        let s = SignIns::nuovo(&d.percorso().join("t.json.signins.json"), move || o2.get());
        let caps = Json::analizza(r#"["mail","calendar","mail"]"#).unwrap();
        let j = s.start(" A@B.it ", &caps).unwrap();
        assert_eq!(j["capabilities"].in_js(), r#"["mail","calendar"]"#);
        assert_eq!(s.start("a@b.it", &Json::analizza(r#"["chat"]"#).unwrap()).unwrap_err().to_string(), "A connection is already in progress. Finish it before changing services.");
        assert_eq!(s.start("a@b.it", &Json::analizza(r#"["calendar","mail"]"#).unwrap()).unwrap()["id"], j["id"]);
        assert_eq!(s.start("a b@c", &caps).unwrap_err().to_string(), "Valid account address required");
        let u = s.update(campo(&j, "id"), &[("phase", Some(Json::from("browser"))), ("slot", Some(Json::from(3))), ("error", None)]).unwrap();
        assert_eq!(campo(&u, "phase"), "browser");
        assert_eq!(u.get("error"), Some(&Json::Null));
        ora.set(1301);
        let l = s.list().unwrap();
        assert_eq!((campo(&l[0], "phase"), campo(&l[0], "error"), l[0]["ended"].in_js()), ("blocked", "Sign-in timed out. Resume when ready.", "1300".to_string()));
        let c = s.cancel(&j["id"], &Json::from("a@b.it")).unwrap();
        assert_eq!((campo(&c, "error"), c["ended"].in_js()), (CANCELLED, "1300".to_string()));
        let testo = std::fs::read_to_string(d.percorso().join("t.json.signins.json")).unwrap();
        assert!(testo.starts_with(&format!(r#"[{{"id":"{}","account":"a@b.it","capabilities":["mail","calendar"],"phase":"blocked","created":1000,"slot":3,"error":"Connection cancelled.","ended":1300}}"#, campo(&j, "id"))), "{testo}");
    }
}
