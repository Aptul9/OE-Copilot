//! What the tests of the light agent share: a network that records the requests and answers as the test says (the
//! stubbed fetch of the TypeScript tests), temporary folders, a runtime of one thread.

use super::http::{Http, Request, Response};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A request as the fake network saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("JSON body")
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

type Answer = Rc<dyn Fn(&Seen) -> Result<Response, String>>;

/// The network of a test: every request recorded, answered by the last answer given.
#[derive(Clone)]
pub struct Net {
    seen: Rc<RefCell<Vec<Seen>>>,
    answer: Rc<RefCell<Answer>>,
}

impl Net {
    pub fn new() -> Net {
        let none: Answer = Rc::new(|s: &Seen| Err(format!("unexpected request {} {}", s.method, s.url)));
        Net { seen: Rc::default(), answer: Rc::new(RefCell::new(none)) }
    }

    /// Status and body of the answer to each request.
    pub fn answer(&self, f: impl Fn(&Seen) -> (u16, String) + 'static) {
        *self.answer.borrow_mut() = Rc::new(move |s| {
            let (status, body) = f(s);
            Ok(Response { status, headers: vec![], body: body.into_bytes() })
        });
    }

    /// The whole answer, headers included.
    pub fn respond(&self, f: impl Fn(&Seen) -> Response + 'static) {
        *self.answer.borrow_mut() = Rc::new(move |s| Ok(f(s)));
    }

    /// Every request fails with this text, as when nothing answers.
    pub fn fail(&self, text: &'static str) {
        *self.answer.borrow_mut() = Rc::new(move |_| Err(text.to_string()));
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.borrow().clone()
    }

    pub fn clear(&self) {
        self.seen.borrow_mut().clear();
    }

    pub fn http(&self) -> Http {
        let (seen, answer) = (self.seen.clone(), self.answer.clone());
        Rc::new(move |r: Request| {
            let s = Seen { method: r.method.to_string(), url: r.url, headers: r.headers, body: r.body };
            seen.borrow_mut().push(s.clone());
            let f = answer.borrow().clone();
            let out = f(&s);
            Box::pin(async move { out })
        })
    }
}

pub fn response(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
    Response { status, headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body: body.as_bytes().to_vec() }
}

static DIRS: AtomicUsize = AtomicUsize::new(0);

pub fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("trweb-agent-{}-{}", std::process::id(), DIRS.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

pub fn run<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().enable_io().build().expect("runtime");
    tokio::task::LocalSet::new().block_on(&rt, f)
}
