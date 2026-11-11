use crate::file::{leggi_utf8, scrivi_atomico};
use oe_nucleo::errori::Errore;
use oe_nucleo::json::{Json, Mappa};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct SharedCommand {
    pub id: String,
    pub account: String,
    /// "remove", "chat" o "calls".
    pub operation: String,
    pub on: Option<bool>,
    pub error: Option<String>,
}

impl SharedCommand {
    pub fn da_json(v: &Json) -> SharedCommand {
        let testo = |k: &str| v.get(k).and_then(Json::as_str).map(str::to_string);
        SharedCommand {
            id: testo("id").unwrap_or_default(),
            account: testo("account").unwrap_or_default(),
            operation: testo("operation").unwrap_or_default(),
            on: v.get("on").and_then(Json::as_bool),
            error: testo("error"),
        }
    }

    pub fn a_json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("id", &self.id);
        m.insert("account", &self.account);
        m.insert("operation", &self.operation);
        if let Some(on) = self.on {
            m.insert("on", on);
        }
        if let Some(e) = &self.error {
            m.insert("error", e);
        }
        Json::Obj(m)
    }
}

/// Un file che manca e' un elenco vuoto; un file che non si legge o non e' un elenco e' un errore.
pub fn read_commands(file: &Path) -> Result<Vec<SharedCommand>, Errore> {
    let testo = match leggi_utf8(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Errore::Altro(e.to_string())),
    };
    match Json::analizza(&testo).map_err(|e| Errore::Altro(e.messaggio))? {
        Json::Arr(v) => Ok(v.iter().map(SharedCommand::da_json).collect()),
        _ => Err(Errore::Tipo("commands.map is not a function".into())),
    }
}

pub fn save_commands(file: &Path, commands: &[SharedCommand]) -> Result<(), Errore> {
    let json = Json::Arr(commands.iter().map(SharedCommand::a_json).collect());
    scrivi_atomico(file, &json.in_js(), None)
}

#[cfg(test)]
mod prove {
    use super::*;
    use crate::prova_util::CartellaTemp;

    #[test]
    fn giro_su_disco() {
        let d = CartellaTemp::nuova();
        let f = d.percorso().join("c.json");
        assert_eq!(read_commands(&f).unwrap(), vec![]);
        let c = vec![
            SharedCommand { id: "1".into(), account: "a@b".into(), operation: "chat".into(), on: Some(true), error: None },
            SharedCommand { id: "2".into(), account: "c@d".into(), operation: "remove".into(), on: None, error: Some("x".into()) },
        ];
        save_commands(&f, &c).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), r#"[{"id":"1","account":"a@b","operation":"chat","on":true},{"id":"2","account":"c@d","operation":"remove","error":"x"}]"#);
        assert_eq!(read_commands(&f).unwrap(), c);
        std::fs::write(&f, "{").unwrap();
        assert!(read_commands(&f).is_err());
    }
}
