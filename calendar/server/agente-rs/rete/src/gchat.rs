//! Binary Google Chat RPCs used by gweb. No browser, stream listener, implicit read receipts or send retries.
//!
//! Lo schema (proto/googlechat.proto) e i parametri delle RPC vengono da gweb e dalla sua `gchatmeow`, distribuita sotto AGPL-3.0: la
//! provenienza e la licenza sono in proto/NOTICE.md e proto/AGPL-3.0.txt.

use crate::google_web::{ErroreGoogle, SessioneCondivisa, assorbi_cookie, intestazione_cookie, leggi_sessione_importata, ruota_cookie};
use crate::js::{lunghezza_utf16, spazio_js};
use crate::protobuf::{codifica_chat, decodifica_chat};
use crate::sessione_web::modulo;
use crate::trasporto::{Richiesta, Risposta, Trasporto};
use oe_nucleo::json::{Json, Mappa, numero_js};
use regex::Regex;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

const BASE: &str = "https://chat.google.com/u/0";
const API_KEY: &str = "AIzaSyD7InnYR3VKdb4j2rMUEbTCIr2VyEazl6k";
pub const CHAT_MAX_BYTES: usize = 8 * 1024 * 1024;
const ATTACHMENT_MAX_BYTES: usize = 5 * 1024 * 1024;

type Esito<T> = Result<T, ErroreGoogle>;

fn errore(messaggio: impl Into<String>, status: u16) -> ErroreGoogle {
    ErroreGoogle::http(messaggio, status)
}

fn header() -> Json {
    let mut caps = Mappa::new();
    caps.insert("spam_room_invites_level", "FULLY_SUPPORTED");
    let mut h = Mappa::new();
    h.insert("client_type", "WEB");
    h.insert("client_version", "2440378181258");
    h.insert("client_feature_capabilities", Json::Obj(caps));
    Json::Obj(h)
}

/// `obj(v)`: l'oggetto, o uno vuoto.
fn obj(v: Option<&Json>) -> Mappa {
    match v {
        Some(Json::Obj(m)) => m.clone(),
        _ => Mappa::new(),
    }
}

/// `array(v)`: gli elementi come oggetti (quelli che non lo sono diventano vuoti).
fn array(v: Option<&Json>) -> Vec<Mappa> {
    match v {
        Some(Json::Arr(a)) => a.iter().map(|x| obj(Some(x))).collect(),
        _ => Vec::new(),
    }
}

/// `text(v)`: stringhe e numeri come testo, il resto "".
fn text(v: Option<&Json>) -> String {
    match v {
        Some(Json::Str(s)) => s.clone(),
        Some(Json::Num(n)) => numero_js(*n),
        _ => String::new(),
    }
}

/// `Number(v)` di un valore decodificato (gli int64 arrivano come testo).
fn numero(v: Option<&Json>) -> f64 {
    match v {
        None => f64::NAN,
        Some(x) => crate::js::numero_di(x),
    }
}

fn verita(v: Option<&Json>) -> bool {
    v.is_some_and(crate::js::vero)
}

fn mappa(coppie: Vec<(&str, Json)>) -> Mappa {
    let mut m = Mappa::new();
    for (k, v) in coppie {
        m.insert(k, v);
    }
    m
}

/// `/^[A-Za-z0-9_%=-]{1,256}$/`
fn id(v: Option<&Json>, nome: &str) -> Esito<String> {
    match v {
        Some(Json::Str(s)) if (1..=256).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_%=-".contains(&b)) => Ok(s.clone()),
        _ => Err(errore(format!("invalid {nome}"), 400)),
    }
}

fn group(conversazione: &str, kind: &str) -> Esito<Json> {
    id(Some(&Json::from(conversazione)), "conversation")?;
    Ok(Json::Obj(if kind == "dm" {
        mappa(vec![("dm_id", Json::Obj(mappa(vec![("dm_id", Json::from(conversazione))])))])
    } else {
        mappa(vec![("space_id", Json::Obj(mappa(vec![("space_id", Json::from(conversazione))])))])
    }))
}

/// `new Date(micros / 1000).toISOString()`, "" fuori dall'intervallo delle date di JavaScript.
pub fn iso(micros: Option<&Json>) -> String {
    let ms = numero(Some(&Json::from(text(micros)))) / 1000.0;
    if !(ms.is_finite() && ms >= 0.0 && ms < 8.64e15) {
        return String::new();
    }
    let ms = ms.trunc() as i64;
    let (giorni, resto) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (a, m, g) = oe_nucleo::tempo::civile_da_giorni(giorni);
    let anno = if (0..=9999).contains(&a) { format!("{a:04}") } else { format!("+{a:06}") };
    format!("{anno}-{m:02}-{g:02}T{:02}:{:02}:{:02}.{:03}Z", resto / 3_600_000, resto / 60_000 % 60, resto / 1000 % 60, resto % 1000)
}

/// L'indirizzo, se è di Google Chat, degli accessi o dei contenuti di Google; altrimenti un 400.
fn trusted_url(grezzo: &str) -> Esito<url::Url> {
    let u = url::Url::parse(grezzo).map_err(|_| errore("invalid Google URL", 400))?;
    let host = u.host_str().unwrap_or_default();
    let ammesso = ["chat.google.com", "accounts.google.com", "chat-wsx.usercontent.google.com"].contains(&host) || host.ends_with(".googleusercontent.com");
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() || u.fragment().is_some_and(|f| !f.is_empty()) || u.port().is_some() || !ammesso {
        return Err(errore("Google URL host not allowed", 400));
    }
    Ok(u)
}

/// Il corpo, al massimo `max` byte (anche solo dichiarati).
fn byte_di(r: &Risposta, max: usize) -> Esito<Vec<u8>> {
    let dichiarata = r.intestazioni.primo("content-length").map_or(f64::NAN, |v| crate::js::numero_di(&Json::from(v)));
    if dichiarata > max as f64 || r.oltre_max.is_some() || r.corpo.len() > max {
        return Err(errore("Google response exceeds size limit", 502));
    }
    Ok(r.corpo.clone())
}

fn base64_standard() -> base64::engine::GeneralPurpose {
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
    GeneralPurpose::new(&base64::alphabet::STANDARD, GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent).with_decode_allow_trailing_bits(true))
}

/// `/^[A-Za-z0-9+/]+={0,2}$/` (con `vuoto` anche senza caratteri prima dell'uguale).
fn forma_base64(s: &str, vuoto: bool) -> bool {
    let corpo = s.trim_end_matches('=');
    s.len() - corpo.len() <= 2 && (vuoto || !corpo.is_empty()) && corpo.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

/// `Buffer.toString("ascii")`: di ogni byte i sette bit bassi.
fn ascii(b: &[u8]) -> String {
    b.iter().map(|x| char::from(x & 0x7f)).collect()
}

/// Un esito atteso da più chiamanti: il primo lo calcola, gli altri lo aspettano.
struct Condiviso {
    esito: RefCell<Option<Esito<()>>>,
    fatto: tokio::sync::Notify,
}

impl Condiviso {
    fn nuovo() -> Rc<Self> {
        Rc::new(Condiviso { esito: RefCell::new(None), fatto: tokio::sync::Notify::new() })
    }

    async fn attendi(&self) -> Esito<()> {
        loop {
            let notifica = self.fatto.notified();
            tokio::pin!(notifica);
            notifica.as_mut().enable();
            if let Some(e) = self.esito.borrow().clone() {
                return e;
            }
            notifica.await;
        }
    }

    fn compi(&self, e: Esito<()>) {
        *self.esito.borrow_mut() = Some(e);
        self.fatto.notify_waiters();
    }
}

pub type Salva = Rc<dyn Fn() -> Result<(), ErroreGoogle>>;
pub type Guardia = Rc<dyn Fn() -> Result<bool, ErroreGoogle>>;

#[derive(Clone, Default)]
pub struct OpzioniChat {
    pub now: Option<Rc<dyn Fn() -> i64>>,
    /// I cookie della sessione sono cambiati: chi la tiene la salva; un salvataggio fallito ferma la richiesta.
    pub changed: Option<Salva>,
    /// La sessione è ancora quella dell'archivio: se no ogni richiesta è un 409 e la rotazione dei cookie non parte. Il wrapper di fetchFn
    /// in TypeScript: un errore nel leggere l'archivio diventa quello del trasporto.
    pub guardia: Option<Guardia>,
}

pub struct GoogleChat {
    pub session: SessioneCondivisa,
    trasporto: Rc<dyn Trasporto>,
    now: Rc<dyn Fn() -> i64>,
    changed: Salva,
    guardia: Option<Guardia>,
    xsrf: RefCell<String>,
    xsrf_at: Cell<i64>,
    refreshing: RefCell<Option<Rc<Condiviso>>>,
    renewing: RefCell<Option<Rc<Condiviso>>>,
    io: RefCell<String>,
    names: RefCell<HashMap<String, (String, String)>>,
}

struct Intestazioni<'a>(Vec<(&'a str, String)>);

impl GoogleChat {
    pub fn nuovo(session: SessioneCondivisa, trasporto: Rc<dyn Trasporto>, o: OpzioniChat) -> Esito<Self> {
        let now = o.now.unwrap_or_else(|| Rc::new(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)));
        // Saved files pass the same boundary as imported files before any cookie leaves.
        let dati = session.borrow().in_json();
        let (ua, cookies) = leggi_sessione_importata(&dati, (now() as f64 / 1000.0).floor())?;
        {
            let mut s = session.borrow_mut();
            s.cookies = cookies;
            s.user_agent = ua;
        }
        Ok(GoogleChat {
            session,
            trasporto,
            now,
            changed: o.changed.unwrap_or_else(|| Rc::new(|| Ok(()))),
            guardia: o.guardia,
            xsrf: RefCell::new(String::new()),
            xsrf_at: Cell::new(0),
            refreshing: RefCell::new(None),
            renewing: RefCell::new(None),
            io: RefCell::new(String::new()),
            names: RefCell::new(HashMap::new()),
        })
    }

    fn adesso_s(&self) -> f64 {
        ((self.now)() as f64 / 1000.0).floor()
    }

    async fn fetch(&self, indirizzo: &str, metodo: &str, intestazioni: Intestazioni<'_>, corpo: Option<Vec<u8>>, max: usize) -> Esito<Risposta> {
        let u = trusted_url(indirizzo)?;
        let trasporto_fallito = || errore("Google Chat transport failed; write outcome may be unknown", 502);
        if let Some(g) = &self.guardia {
            if !g().map_err(|_| trasporto_fallito())? {
                return Err(errore("Google Chat account changed", 409));
            }
        }
        let host = u.host_str().unwrap_or_default().to_string();
        let (ua, cookie) = {
            let s = self.session.borrow();
            (s.user_agent.clone(), intestazione_cookie(&s.cookies, &host, u.path(), self.adesso_s()))
        };
        let mut r = Richiesta::get(u.as_str()).metodo(metodo);
        for (k, v) in intestazioni.0 {
            r = r.intestazione(k, v);
        }
        r = r.intestazione("User-Agent", ua);
        if !cookie.is_empty() {
            r = r.intestazione("Cookie", cookie);
        }
        r.corpo = corpo;
        let risposta = self
            .trasporto
            .invia(r.senza_redirect().scadenza(Duration::from_millis(30_000)).max_byte(Some(max)))
            .await
            .map_err(|_| trasporto_fallito())?;
        let righe: Vec<&str> = risposta.intestazioni.tutti("set-cookie").collect();
        let cambiato = assorbi_cookie(&mut self.session.borrow_mut().cookies, &host, &righe, self.adesso_s(), None);
        if cambiato {
            (self.changed)()?;
        }
        Ok(risposta)
    }

    async fn xsrf_once(&self) -> Esito<()> {
        let hs = r#"["h_hs",null,null,[1,0],null,null,"gmail.pinto-server_20230730.06_p0",1,null,[15,38,36,35,26,30,41,18,24,11,21,14,6],null,null,"3Mu86PSulM4.en..es5",0,null,null,[0]]"#;
        let q = modulo(&[("origin", "https://mail.google.com"), ("shell", "9"), ("hl", "en"), ("wfi", "gtn-roster-iframe-id"), ("hs", hs)]);
        let r = self.fetch(&format!("{BASE}/mole/world?{q}"), "GET", Intestazioni(vec![("refer", "https://mail.google.com/".to_string())]), None, CHAT_MAX_BYTES).await?;
        if [401, 302, 303].contains(&r.stato) {
            return Err(errore("Google session expired: import session again", 409));
        }
        if !r.ok() {
            return Err(errore(format!("Google Chat HTTP {}", r.stato), 502));
        }
        let html = String::from_utf8_lossy(&byte_di(&r, CHAT_MAX_BYTES)?).into_owned();
        static WIZ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.WIZ_global_data\s*=\s*(\{(?s:.)*?\});</script>").expect("valida"));
        let non_riconosciuta = || errore("Google Chat session page not recognized", 502);
        let trovato = WIZ.captures(&html).and_then(|c| c.get(1)).ok_or_else(non_riconosciuta)?;
        let dati = obj(Some(&Json::analizza(trovato.as_str()).map_err(|_| non_riconosciuta())?));
        if dati.get("qwAQke").and_then(Json::as_str) == Some("AccountsSignInUi") {
            return Err(errore("Google session expired: import session again", 409));
        }
        let Some(Json::Str(token)) = dati.get("SMqcke").filter(|t| t.as_str().is_some_and(|s| !s.is_empty())) else { return Err(errore("Google Chat XSRF token missing", 502)) };
        *self.xsrf.borrow_mut() = token.clone();
        self.xsrf_at.set((self.now)());
        self.io.borrow_mut().clear();
        Ok(())
    }

    async fn ensure_xsrf(&self, force: bool) -> Esito<()> {
        if !force && !self.xsrf.borrow().is_empty() && (self.now)() - self.xsrf_at.get() < 86_400_000 {
            return Ok(());
        }
        let in_corso = self.refreshing.borrow().clone();
        if let Some(p) = in_corso {
            return p.attendi().await;
        }
        let p = Condiviso::nuovo();
        *self.refreshing.borrow_mut() = Some(p.clone());
        let esito = match self.xsrf_once().await {
            Err(ErroreGoogle::Http { status: 409, .. }) => self.renew_session().await,
            altro => altro,
        };
        *self.refreshing.borrow_mut() = None;
        p.compi(esito.clone());
        esito
    }

    async fn renew_session(&self) -> Esito<()> {
        let in_corso = self.renewing.borrow().clone();
        if let Some(p) = in_corso {
            return p.attendi().await;
        }
        let p = Condiviso::nuovo();
        *self.renewing.borrow_mut() = Some(p.clone());
        // la guardia vale anche qui: una sessione che non è più dell'archivio non ruota (fetchFn di prima solleva, ruotaCookie dà false)
        let vale = self.guardia.as_ref().is_none_or(|g| g().unwrap_or(false));
        let esito = if vale && ruota_cookie(&self.session, &self.trasporto, self.adesso_s()).await { (self.changed)() } else { Ok(()) };
        let esito = match esito {
            Ok(()) => {
                self.io.borrow_mut().clear();
                self.xsrf_once().await
            }
            Err(e) => Err(e),
        };
        *self.renewing.borrow_mut() = None;
        p.compi(esito.clone());
        esito
    }

    fn pinned(&self) -> bool {
        self.session.borrow().self_id.as_deref().is_some_and(|s| !s.is_empty())
    }

    async fn rpc(&self, endpoint: &str, tipo_richiesta: &str, tipo_risposta: &str, dati: Mappa) -> Esito<Mappa> {
        let mut richiesta = dati;
        richiesta.insert("request_header", header());
        let corpo = codifica_chat(tipo_richiesta, &Json::Obj(richiesta)).map_err(ErroreGoogle::Altro)?;
        self.ensure_xsrf(false).await?;
        if endpoint != "get_self_user_status" && self.pinned() {
            self.self_id().await?;
        }
        for riprova in [false, true] {
            let q = modulo(&[("c", "1"), ("rt", "b"), ("alt", "proto"), ("key", API_KEY)]);
            let intestazioni = Intestazioni(vec![
                ("Content-Type", "application/x-protobuf".to_string()),
                ("X-Goog-Encode-Response-If-Executable", "base64".to_string()),
                ("x-framework-xsrf-token", self.xsrf.borrow().clone()),
            ]);
            let r = self.fetch(&format!("{BASE}/api/{endpoint}?{q}"), "POST", intestazioni, Some(corpo.clone()), CHAT_MAX_BYTES).await?;
            if r.stato == 401 {
                if riprova {
                    return Err(errore("Google session rejected: import session again", 409));
                }
                self.renew_session().await?;
                if endpoint != "get_self_user_status" && self.pinned() {
                    self.self_id().await?;
                }
                continue;
            }
            if !r.ok() {
                let status = if [400, 403, 404, 429].contains(&r.stato) { r.stato } else { 502 };
                return Err(errore(format!("Google Chat HTTP {}", r.stato), status));
            }
            let dati = byte_di(&r, CHAT_MAX_BYTES)?;
            if let Ok(Json::Obj(m)) = decodifica_chat(tipo_risposta, &dati) {
                return Ok(m);
            }
            let invalido = || errore("Google Chat returned invalid protobuf; write outcome may be unknown", 502);
            let testo = ascii(&dati);
            let codificato = testo.trim_matches(spazio_js);
            if !forma_base64(codificato, false) {
                return Err(invalido());
            }
            use base64::Engine;
            let grezzo = base64_standard().decode(codificato).map_err(|_| invalido())?;
            return match decodifica_chat(tipo_risposta, &grezzo) {
                Ok(Json::Obj(m)) => Ok(m),
                _ => Err(invalido()),
            };
        }
        Err(errore("Google session rejected", 409))
    }

    fn self_id(&self) -> Pin<Box<dyn Future<Output = Esito<String>> + '_>> {
        Box::pin(async move {
            if self.io.borrow().is_empty() {
                let r = self.rpc("get_self_user_status", "GetSelfUserStatusRequest", "GetSelfUserStatusResponse", Mappa::new()).await?;
                let io = text(obj(obj(r.get("user_status")).get("user_id")).get("id"));
                *self.io.borrow_mut() = io.clone();
                if io.is_empty() {
                    return Err(errore("Google Chat identity missing", 502));
                }
            }
            let io = self.io.borrow().clone();
            if self.pinned() && self.session.borrow().self_id.as_deref() != Some(io.as_str()) {
                return Err(errore("Google Chat session belongs to another account", 409));
            }
            Ok(io)
        })
    }

    async fn resolve(&self, ids: Vec<String>) -> Esito<()> {
        let mut servono: Vec<String> = Vec::new();
        for i in ids {
            if !i.is_empty() && !servono.contains(&i) && !self.names.borrow().contains_key(&i) {
                servono.push(i);
            }
        }
        for gruppo in servono.chunks(40) {
            let membri: Vec<Json> = gruppo.iter().map(|i| Json::Obj(mappa(vec![("user_id", Json::Obj(mappa(vec![("id", Json::from(i.as_str()))])))]))).collect();
            let r = self.rpc("get_members", "GetMembersRequest", "GetMembersResponse", mappa(vec![("member_ids", Json::Arr(membri))])).await?;
            for membro in array(r.get("members")) {
                let user = obj(membro.get("user"));
                let user_id = text(obj(user.get("user_id")).get("id"));
                if !user_id.is_empty() {
                    let email = text(user.get("email"));
                    let nome = [text(user.get("name")), email.clone(), user_id.clone()].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
                    self.names.borrow_mut().insert(user_id, (nome, email.to_lowercase()));
                }
            }
        }
        Ok(())
    }

    fn nome(&self, id: &str) -> String {
        self.names.borrow().get(id).map_or_else(|| id.to_string(), |n| n.0.clone())
    }

    pub async fn identity(&self) -> Esito<(String, String)> {
        let io = self.self_id().await?;
        self.resolve(vec![io.clone()]).await?;
        let account = self.names.borrow().get(&io).map(|n| n.1.clone()).unwrap_or_default();
        if account.is_empty() || account != self.session.borrow().account.to_lowercase() {
            return Err(errore("Google Chat session belongs to another account or has no email", 409));
        }
        Ok((account, io))
    }

    pub async fn list(&self) -> Esito<Json> {
        let mut sezione = Mappa::new();
        sezione.insert("page_size", 200);
        let world = self
            .rpc(
                "paginated_world",
                "PaginatedWorldRequest",
                "PaginatedWorldResponse",
                mappa(vec![("fetch_from_user_spaces", Json::Bool(true)), ("fetch_options", Json::Arr(vec![Json::from("EXCLUDE_GROUP_LITE")])), ("world_section_requests", Json::Arr(vec![Json::Obj(sezione)]))]),
            )
            .await?;
        let io = self.self_id().await?;
        let elementi = array(world.get("world_items"));
        let membri = |item: &Mappa| array(obj(item.get("dm_members")).get("members")).iter().map(|u| text(u.get("id"))).collect::<Vec<_>>();
        self.resolve(elementi.iter().flat_map(membri).collect()).await?;
        let mut conversazioni: Vec<Mappa> = Vec::new();
        for item in &elementi {
            let g = obj(item.get("group_id"));
            let dm = verita(g.get("dm_id"));
            let id = text(if dm { obj(g.get("dm_id")).get("dm_id").cloned() } else { obj(g.get("space_id")).get("space_id").cloned() }.as_ref());
            let nome = if dm {
                membri(item).iter().filter(|p| **p != io).map(|p| self.nome(p)).collect::<Vec<_>>().join(", ")
            } else {
                Some(text(item.get("room_name"))).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone())
            };
            let non_letti = obj(item.get("read_state")).get("unread_message_count").filter(|v| !v.is_null()).map_or(0.0, |v| crate::js::numero_di(v));
            if id.is_empty() {
                continue;
            }
            conversazioni.push(mappa(vec![
                ("id", Json::from(id)),
                ("kind", Json::from(if dm { "dm" } else { "space" })),
                ("name", Json::from(nome)),
                ("unread", Json::from(non_letti)),
                ("last", Json::from(iso(item.get("sort_timestamp")))),
            ]));
        }
        let ultimo = |m: &Mappa| m.get("last").and_then(Json::as_str).unwrap_or_default().to_string();
        conversazioni.sort_by(|a, b| ultimo(b).cmp(&ultimo(a)));
        Ok(Json::Obj(mappa(vec![
            ("conversations", Json::Arr(conversazioni.into_iter().map(Json::Obj).collect())),
            ("self_id", Json::from(io)),
            ("coverage", Json::from("recent-conversations")),
            ("pagination", Json::Arr(array(world.get("world_section_responses")).into_iter().map(Json::Obj).collect())),
        ])))
    }

    pub async fn read(&self, conversazione: &str, kind: &str, topics: f64) -> Esito<Json> {
        if topics.fract() != 0.0 || !(1.0..=1000.0).contains(&topics) {
            return Err(errore("topics must be 1..1000", 400));
        }
        let r = self
            .rpc(
                "list_topics",
                "ListTopicsRequest",
                "ListTopicsResponse",
                mappa(vec![
                    ("group_id", group(conversazione, kind)?),
                    ("page_size_for_topics", Json::from(topics)),
                    ("page_size_for_unread_replies", Json::from(100)),
                    ("page_size_for_read_replies", Json::from(20)),
                ]),
            )
            .await?;
        let righe: Vec<(Mappa, String)> = array(r.get("topics")).iter().flat_map(|t| array(t.get("replies")).into_iter().map(move |m| (m, text(obj(t.get("id")).get("topic_id"))))).collect();
        let autore = |m: &Mappa| text(obj(obj(m.get("creator")).get("user_id")).get("id"));
        self.resolve(righe.iter().map(|(m, _)| autore(m)).collect()).await?;
        let mut messaggi: Vec<Mappa> = righe
            .iter()
            .map(|(m, thread)| {
                let id = text(obj(m.get("id")).get("message_id"));
                let da = autore(m);
                let cancellato = numero(m.get("delete_time")) > 0.0;
                let annotazioni = array(m.get("annotations"));
                let allegati: Vec<Json> = annotazioni
                    .iter()
                    .filter(|a| verita(a.get("upload_metadata")))
                    .map(|a| {
                        let u = obj(a.get("upload_metadata"));
                        Json::Obj(mappa(vec![("token", Json::from(text(u.get("attachment_token")))), ("name", Json::from(text(u.get("content_name")))), ("mime_type", Json::from(text(u.get("content_type"))))]))
                    })
                    .collect();
                let elementi: Vec<Json> = annotazioni
                    .iter()
                    .filter(|a| verita(a.get("drive_metadata")) || verita(a.get("url_metadata")))
                    .map(|a| Json::Obj(obj(a.get("drive_metadata").filter(|v| !v.is_null()).or(a.get("url_metadata")))))
                    .collect();
                mappa(vec![
                    ("id", Json::from(id.as_str())),
                    ("thread", Json::from(thread.as_str())),
                    ("from_id", Json::from(da.as_str())),
                    ("from", Json::from(self.nome(&da))),
                    ("time", Json::from(iso(m.get("create_time")))),
                    ("text", Json::from(if cancellato { "[deleted]".to_string() } else { text(m.get("text_body")) })),
                    ("reply", Json::Bool(id != *thread)),
                    ("edited", Json::Bool(numero(m.get("last_edit_time")) > 0.0)),
                    ("deleted", Json::Bool(cancellato)),
                    ("attachments", Json::Arr(allegati)),
                    ("items", Json::Arr(elementi)),
                    ("reactions", Json::Arr(array(m.get("reactions")).into_iter().map(Json::Obj).collect())),
                ])
            })
            .collect();
        let ora = |m: &Mappa| m.get("time").and_then(Json::as_str).unwrap_or_default().to_string();
        messaggi.sort_by_key(ora);
        let io = self.self_id().await?;
        Ok(Json::Obj(mappa(vec![
            ("messages", Json::Arr(messaggi.into_iter().map(Json::Obj).collect())),
            ("self_id", Json::from(io)),
            ("coverage", Json::from("recent-topics")),
            ("limits", Json::Obj(mappa(vec![("topics", Json::from(topics)), ("unread_replies", Json::from(100)), ("read_replies", Json::from(20))]))),
        ])))
    }

    pub async fn search(&self, query: &str, topics: f64) -> Esito<Json> {
        if query.is_empty() || lunghezza_utf16(query) > 1024 {
            return Err(errore("query must be 1..1024 characters", 400));
        }
        let elenco = self.list().await?;
        let cercato = query.to_lowercase();
        let mut trovati = Vec::new();
        for c in elenco["conversations"].as_arr().map(Vec::as_slice).unwrap_or_default() {
            let letti = self.read(c["id"].as_str().unwrap_or_default(), c["kind"].as_str().unwrap_or_default(), topics).await?;
            for m in letti["messages"].as_arr().map(Vec::as_slice).unwrap_or_default() {
                if m["text"].as_str().unwrap_or_default().to_lowercase().contains(&cercato) {
                    trovati.push(Json::Obj(mappa(vec![("conversation", c.clone()), ("message", m.clone())])));
                }
            }
        }
        Ok(Json::Obj(mappa(vec![("hits", Json::Arr(trovati)), ("coverage", Json::from("local-search-recent-topics")), ("topics_per_conversation", Json::from(topics))])))
    }

    pub async fn details(&self, conversazione: &str, kind: &str) -> Esito<Json> {
        self.self_id().await?;
        let opzioni = Json::Arr(vec![Json::from("MEMBERS"), Json::from("MEMBER_IDS_ONLY")]);
        Ok(Json::Obj(self.rpc("get_group", "GetGroupRequest", "GetGroupResponse", mappa(vec![("group_id", group(conversazione, kind)?), ("fetch_options", opzioni)])).await?))
    }

    pub async fn download(&self, indirizzo: &str) -> Esito<Json> {
        let mut indirizzo = indirizzo.to_string();
        for _ in 0..8 {
            if trusted_url(&indirizzo)?.host_str() == Some("accounts.google.com") {
                return Err(errore("Google attachment sign-in required", 409));
            }
            let r = self.fetch(&indirizzo, "GET", Intestazioni(Vec::new()), None, ATTACHMENT_MAX_BYTES).await?;
            if [301, 302, 303, 307, 308].contains(&r.stato) {
                let Some(location) = r.intestazioni.ottieni("location") else { return Err(errore("Google attachment redirect missing URL", 502)) };
                let base = url::Url::parse(&indirizzo).map_err(|e| ErroreGoogle::Altro(e.to_string()))?;
                let dopo = base.join(&location).map_err(|e| ErroreGoogle::Altro(format!("Invalid URL: {e}")))?;
                indirizzo = trusted_url(dopo.as_str())?.to_string();
                continue;
            }
            if !r.ok() {
                return Err(errore(format!("Google attachment HTTP {}", r.stato), 502));
            }
            use base64::Engine;
            let dati = base64::engine::general_purpose::STANDARD.encode(byte_di(&r, ATTACHMENT_MAX_BYTES)?);
            let mime = r.intestazioni.ottieni("content-type").unwrap_or_else(|| "application/octet-stream".to_string());
            return Ok(Json::Obj(mappa(vec![("data", Json::from(dati)), ("mime_type", Json::from(mime))])));
        }
        Err(errore("too many Google attachment redirects", 502))
    }

    pub async fn attachment(&self, token: &str, mime_type: &str) -> Esito<Json> {
        if token.is_empty() || lunghezza_utf16(token) > 8192 || lunghezza_utf16(mime_type) > 256 {
            return Err(errore("invalid attachment", 400));
        }
        let immagine = mime_type.starts_with("image/");
        let mut q = vec![("attachment_token", token), ("url_type", if immagine { "FIFE_URL" } else { "DOWNLOAD_URL" })];
        if immagine {
            q.push(("sz", "w10000-h10000"));
            q.push(("content_type", mime_type));
        }
        self.download(&format!("https://chat.google.com/api/get_attachment_url?{}", modulo(&q))).await
    }

    async fn upload(&self, conversazione: &str, file: &Mappa) -> Esito<Json> {
        let nome_valido = |n: &str| (1..=180).contains(&lunghezza_utf16(n)) && !n.chars().any(|c| (c as u32) < 0x20 || c == '\x7f' || c == '/' || c == '\\');
        let (Some(Json::Str(nome)), Some(Json::Str(dati))) = (file.get("name"), file.get("data")) else { return Err(errore("invalid file", 400)) };
        if !nome_valido(nome) || !forma_base64(dati, true) {
            return Err(errore("invalid file", 400));
        }
        use base64::Engine;
        let dati = base64_standard().decode(dati).unwrap_or_default();
        if dati.len() > ATTACHMENT_MAX_BYTES {
            return Err(errore("file exceeds size limit", 413));
        }
        static MIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.+-]+/[A-Za-z0-9_.+-]+$").expect("valida"));
        let mime = file.get("mime_type").and_then(Json::as_str).filter(|m| MIME.is_match(m)).unwrap_or("application/octet-stream").to_string();
        self.ensure_xsrf(false).await?;
        self.self_id().await?;
        let inizio = Intestazioni(vec![
            ("x-goog-upload-protocol", "resumable".to_string()),
            ("x-goog-upload-command", "start".to_string()),
            ("x-goog-upload-content-length", dati.len().to_string()),
            ("x-goog-upload-content-type", mime),
            ("x-goog-upload-file-name", crate::sessione_web::codifica_componente(nome)),
            ("x-framework-xsrf-token", self.xsrf.borrow().clone()),
        ]);
        let r = self.fetch(&format!("https://chat.google.com/uploads?{}", modulo(&[("group_id", conversazione)])), "POST", inizio, None, CHAT_MAX_BYTES).await?;
        let dopo = r.intestazioni.ottieni("x-goog-upload-url").filter(|n| !n.is_empty());
        let Some(dopo) = dopo.filter(|_| r.ok()) else { return Err(errore("Google upload start failed", 502)) };
        if trusted_url(&dopo)?.host_str() != Some("chat.google.com") {
            return Err(errore("Google upload start failed", 502));
        }
        self.ensure_xsrf(false).await?;
        self.self_id().await?;
        let fine = Intestazioni(vec![
            ("x-goog-upload-command", "upload, finalize".to_string()),
            ("x-goog-upload-protocol", "resumable".to_string()),
            ("x-goog-upload-offset", "0".to_string()),
            ("x-framework-xsrf-token", self.xsrf.borrow().clone()),
        ]);
        let fatto = self.fetch(&dopo, "PUT", fine, Some(dati), CHAT_MAX_BYTES).await?;
        if !fatto.ok() {
            return Err(errore(format!("Google upload HTTP {}", fatto.stato), 502));
        }
        let invalido = || errore("Google upload returned invalid metadata", 502);
        let testo: String = ascii(&byte_di(&fatto, CHAT_MAX_BYTES)?).chars().filter(|c| !spazio_js(*c)).collect();
        let grezzo = base64_standard().decode(testo.as_bytes()).map_err(|_| invalido())?;
        let metadati = match decodifica_chat("UploadMetadata", &grezzo) {
            Ok(Json::Obj(m)) => m,
            _ => return Err(invalido()),
        };
        if !verita(metadati.get("attachment_token")) {
            return Err(errore("Google upload returned no attachment token", 502));
        }
        Ok(Json::Obj(mappa(vec![("type", Json::from("UPLOAD_METADATA")), ("upload_metadata", Json::Obj(metadati))])))
    }

    pub async fn write(&self, data: &Mappa) -> Esito<Json> {
        if data.get("confirm") != Some(&Json::Bool(true)) {
            return Err(errore("confirm: true required", 400));
        }
        let campi: &[&str] = match data.get("action").and_then(Json::as_str) {
            Some("send") => &["text", "thread", "files"],
            Some("mark_read") => &[],
            Some("edit") => &["message_id", "thread", "text", "topics"],
            Some("delete") => &["message_id", "thread", "topics"],
            Some("react") => &["message_id", "thread", "emoji", "remove", "topics"],
            _ => return Err(errore("unknown Chat action", 400)),
        };
        let azione = data.get("action").and_then(Json::as_str).unwrap_or_default();
        if data.keys().any(|k| !["account", "action", "confirm", "conversation", "kind"].contains(&k.as_str()) && !campi.contains(&k.as_str())) {
            return Err(errore("unexpected Chat write field", 400));
        }
        let conversazione = id(data.get("conversation"), "conversation")?;
        let kind = match data.get("kind").and_then(Json::as_str) {
            Some(k @ ("space" | "dm")) => k,
            _ => return Err(errore("kind must be space or dm", 400)),
        };
        let g = group(&conversazione, kind)?;
        let info = || Json::Obj(mappa(vec![("accept_format_annotations", Json::Bool(true))]));
        let tipo_errato = |k: &str, ok: fn(&Json) -> bool| data.get(k).is_some_and(|v| !ok(v));
        if tipo_errato("text", |v| matches!(v, Json::Str(_)))
            || tipo_errato("files", |v| matches!(v, Json::Arr(_)))
            || tipo_errato("thread", |v| matches!(v, Json::Str(_)))
            || tipo_errato("remove", |v| matches!(v, Json::Bool(_)))
        {
            return Err(errore("invalid Chat write field type", 400));
        }
        self.self_id().await?;
        let topics = match data.get("topics") {
            Some(Json::Num(n)) => *n,
            _ => 100.0,
        };
        if azione == "send" {
            let corpo = data.get("text").and_then(Json::as_str).unwrap_or_default().to_string();
            let file = array(data.get("files"));
            if (corpo.is_empty() && file.is_empty()) || lunghezza_utf16(&corpo) > 32000 || file.len() > 10 {
                return Err(errore("empty or oversized message", 400));
            }
            let mut annotazioni = Vec::new();
            for f in &file {
                annotazioni.push(self.upload(&conversazione, f).await?);
            }
            let mut casuale = [0u8; 8];
            let _ = getrandom::fill(&mut casuale);
            let local_id = format!("oe%{}", casuale.iter().map(|b| format!("{b:02x}")).collect::<String>());
            let mut base = mappa(vec![("local_id", Json::from(local_id)), ("text_body", Json::from(corpo)), ("annotations", Json::Arr(annotazioni)), ("message_info", info())]);
            if verita(data.get("thread")) {
                let thread = id(data.get("thread"), "thread")?;
                base.insert("parent_id", Json::Obj(mappa(vec![("topic_id", Json::Obj(mappa(vec![("group_id", g), ("topic_id", Json::from(thread))])))])));
                return Ok(Json::Obj(self.rpc("create_message", "CreateMessageRequest", "CreateMessageResponse", base).await?));
            }
            base.insert("group_id", g);
            base.insert("history_v2", true);
            return Ok(Json::Obj(self.rpc("create_topic", "CreateTopicRequest", "CreateTopicResponse", base).await?));
        }
        if azione == "mark_read" {
            let quando = ((self.now)() as f64 * 1000.0).to_string();
            return Ok(Json::Obj(self.rpc("mark_group_readstate", "MarkGroupReadstateRequest", "MarkGroupReadstateResponse", mappa(vec![("id", g), ("last_read_time", Json::from(quando))])).await?));
        }
        let message_id = id(data.get("message_id"), "message_id")?;
        let mut thread = match data.get("thread") {
            Some(Json::Str(_)) => id(data.get("thread"), "thread")?,
            _ => String::new(),
        };
        let mut proprio: Option<Json> = None;
        if thread.is_empty() || azione == "edit" || azione == "delete" {
            let letti = self.read(&conversazione, kind, topics).await?;
            proprio = letti["messages"].as_arr().and_then(|m| m.iter().find(|x| x["id"].as_str() == Some(message_id.as_str())).cloned());
            let Some(p) = &proprio else { return Err(errore("message not found in recent topics", 404)) };
            thread = p["thread"].as_str().unwrap_or_default().to_string();
        }
        let messaggio = Json::Obj(mappa(vec![
            ("parent_id", Json::Obj(mappa(vec![("topic_id", Json::Obj(mappa(vec![("group_id", g), ("topic_id", Json::from(thread))])))]))),
            ("message_id", Json::from(message_id)),
        ]));
        if azione == "edit" || azione == "delete" {
            let da = proprio.as_ref().and_then(|p| p["from_id"].as_str().map(str::to_string));
            if da.as_deref() != Some(self.self_id().await?.as_str()) {
                return Err(errore("only own messages can be edited or deleted", 403));
            }
            if azione == "delete" {
                return Ok(Json::Obj(self.rpc("delete_message", "DeleteMessageRequest", "DeleteMessageResponse", mappa(vec![("message_id", messaggio)])).await?));
            }
            let testo = match data.get("text") {
                Some(Json::Str(t)) if !t.is_empty() && lunghezza_utf16(t) <= 32000 => t.clone(),
                _ => return Err(errore("invalid message text", 400)),
            };
            return Ok(Json::Obj(self.rpc("edit_message", "EditMessageRequest", "EditMessageResponse", mappa(vec![("message_id", messaggio), ("text_body", Json::from(testo)), ("message_info", info())])).await?));
        }
        if azione == "react" {
            let emoji = match data.get("emoji") {
                Some(Json::Str(e)) if !e.is_empty() && lunghezza_utf16(e) <= 64 => e.clone(),
                _ => return Err(errore("invalid reaction", 400)),
            };
            let tipo = if data.get("remove") == Some(&Json::Bool(true)) { "REMOVE" } else { "ADD" };
            return Ok(Json::Obj(
                self.rpc(
                    "update_reaction",
                    "UpdateReactionRequest",
                    "UpdateReactionResponse",
                    mappa(vec![("message_id", messaggio), ("emoji", Json::Obj(mappa(vec![("unicode", Json::from(emoji))]))), ("type", Json::from(tipo))]),
                )
                .await?,
            ));
        }
        Err(errore("unknown Chat action", 400))
    }
}
