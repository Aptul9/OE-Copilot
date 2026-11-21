//! Filter untrusted identity headers and retain the local-login configuration.

use oe_http::Richiesta;

pub async fn filtra(r: &mut Richiesta<'_>) {
    r.intestazioni.retain(|(k, _)| !k.starts_with("x-authentik-"));
}

pub fn accesso_con_password() -> bool {
    !crate::config::var("ACCESSO_PASSWORD")
        .is_some_and(|v| crate::py::js_trim(&v).eq_ignore_ascii_case("off"))
}
