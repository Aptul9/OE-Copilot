"""Apertura di una riunione in Microsoft Teams sull'account giusto.

In Teams possono essere collegati piu' account (uno per azienda) e
un link "msteams:" da solo si apre su qualunque account sia attivo in quel
momento. Per pilotare l'account si usa il parametro documentato `tenantId`:
Teams, se il tenant indicato appartiene a un account collegato, ci passa sopra
da solo.

Il tenant si ricava dal dominio della casella (`d.nome@azienda.it`) chiedendolo
all'endpoint pubblico di discovery di Entra ID; il risultato viene messo in
cache su disco perche' e' un dato che non cambia mai.

Si apre SOLO il calendario di quell'account. Il link "meetup-join" non si usa:
manda dritti alla schermata di partecipazione, non alla scheda della riunione
nel calendario (per quella Teams non pubblica nessun deep link, e nemmeno la si
puo' cliccare da fuori: la finestra di Teams non espone il contenuto web
all'accessibilita' di Windows - verificato). Il clic sulla riunione resta a chi
usa l'app; per entrare in riunione c'e' comunque il pulsante "Partecipa".
"""

import json
import logging
import os
import re
import urllib.request

import config

log = logging.getLogger(__name__)

# Id dell'app "Calendario" dentro Teams: e' fisso e uguale per tutti i tenant.
APP_CALENDARIO = "ef56c0de-36fc-4ef8-b417-3d82ba9d073c"

TENANT_CACHE_PATH = os.path.join(config.APP_DIR, "teams_tenants.json")

# Tenant gia' noti, per non dipendere dalla rete al primo click.
TENANT_NOTI = {}


def _cache_leggi():
    try:
        with open(TENANT_CACHE_PATH, "r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, json.JSONDecodeError, TypeError):
        return {}


def _cache_scrivi(dati):
    try:
        config.scrivi_json(TENANT_CACHE_PATH, dati)
    except OSError:
        log.warning("teams_link: cache tenant non scrivibile", exc_info=True)


def dominio(account):
    """Dominio della casella. `account` e' il nome dello store Outlook, che di
    norma coincide con l'indirizzo di posta."""
    m = re.search(r"[\w.+-]+@([\w-]+\.[\w.-]+)", str(account or ""))
    return m.group(1).lower() if m else ""


def tenant_id(account):
    """GUID del tenant Entra ID a cui appartiene la casella, o "" se ignoto."""
    dom = dominio(account)
    if not dom:
        return ""
    if dom in TENANT_NOTI:
        return TENANT_NOTI[dom]
    cache = _cache_leggi()
    if dom in cache:
        return cache[dom] or ""
    guid = ""
    url = (f"https://login.microsoftonline.com/{dom}"
           "/v2.0/.well-known/openid-configuration")
    try:
        with urllib.request.urlopen(url, timeout=6) as r:
            issuer = json.loads(r.read().decode("utf-8")).get("issuer") or ""
        m = re.search(r"([0-9a-f-]{36})", issuer)
        guid = m.group(1) if m else ""
    except Exception:
        # Dominio non su Entra, oppure rete assente: si prosegue senza tenant,
        # il link si apre comunque sull'account attivo.
        log.info("teams_link: tenant di %s non risolto", dom, exc_info=True)
    # Si annota anche l'esito negativo: evita di ritentare a ogni click.
    cache[dom] = guid
    _cache_scrivi(cache)
    return guid


def _con_tenant(url, tid):
    if not tid or "tenantId=" in url:
        return url
    return url + ("&" if "?" in url else "?") + "tenantId=" + tid


def link_calendario(account):
    """Deep link al calendario di Teams dell'account indicato."""
    # L'host ci vuole: "msteams:/l/..." senza "//teams.microsoft.com" fa dire a
    # Teams "si e' verificato un problema durante il raggiungimento dell'app".
    return _con_tenant(
        f"msteams://teams.microsoft.com/l/entity/{APP_CALENDARIO}/calendar",
        tenant_id(account))


def _avvia(url):
    try:
        os.startfile(url)          # noqa: S606 - protocollo msteams: registrato da Teams
        return True
    except OSError:
        log.exception("teams_link: impossibile aprire %s", url)
        return False


def apri(online_url, account):
    """Porta Teams sul calendario dell'account a cui appartiene la riunione."""
    cal = link_calendario(account)
    return {"ok": _avvia(cal), "calendario": cal}
