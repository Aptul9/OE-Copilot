"""Servizio in background: invia i promemoria delle riunioni (Windows + Telegram)."""
import json
import logging
import os
import threading
from datetime import datetime, timedelta

import requests

import config
import eventi as raccolta_eventi

log = logging.getLogger(__name__)


def _parse_dt(s):
    """Converte l'orario ISO prodotto da outlook_com (ora locale) in datetime naive."""
    if not s:
        return None
    s = s.replace("Z", "")
    if "." in s:  # taglia i nanosecondi a 6 cifre per fromisoformat
        head, frac = s.split(".", 1)
        s = head + "." + frac[:6]
    try:
        return datetime.fromisoformat(s)
    except ValueError:
        return None


def _load_fired():
    if os.path.exists(config.FIRED_REMINDERS_PATH):
        try:
            with open(config.FIRED_REMINDERS_PATH, "r", encoding="utf-8") as f:
                return json.load(f)
        except (json.JSONDecodeError, OSError):
            pass
    return {}


def _save_fired(fired):
    # Tiene solo i promemoria dell'ultimo giorno per non far crescere il file.
    cutoff = datetime.now() - timedelta(days=1)
    pruned = {k: v for k, v in fired.items()
              if _parse_dt(v) and _parse_dt(v) > cutoff}
    try:
        config.scrivi_json(config.FIRED_REMINDERS_PATH, pruned)
    except OSError:
        log.warning("promemoria: elenco degli avvisi già inviati non salvato",
                    exc_info=True)


def _send_windows_toast(title, body, url=None):
    try:
        from win11toast import toast
    except ImportError:
        return
    kwargs = {"duration": "long", "app_id": "Calendar Hub"}
    if url:
        kwargs["buttons"] = [{"activationType": "protocol", "arguments": url, "content": "Partecipa"}]
    try:
        toast(title, body, **kwargs)
    except Exception:
        # Una notifica fallita non ferma il servizio, ma va lasciata a verbale.
        log.exception("promemoria: notifica Windows non inviata")


def _send_telegram(cfg, text):
    tg = cfg.get("telegram", {})
    if not tg.get("enabled") or not tg.get("bot_token") or not tg.get("chat_id"):
        return None
    try:
        r = requests.post(
            f"https://api.telegram.org/bot{tg['bot_token']}/sendMessage",
            json={"chat_id": tg["chat_id"], "text": text,
                  "parse_mode": "HTML", "disable_web_page_preview": True},
            timeout=15,
        )
        r.raise_for_status()
        return True
    except requests.RequestException as e:
        return str(e)


def fire_reminder(cfg, ev, minutes):
    """Manda il promemoria sui canali attivi e lo mette a verbale.

    Senza questa riga nel log non c'era modo di sapere se un avviso fosse mai
    partito: si vedeva solo il servizio girare a vuoto.
    """
    log.info("promemoria: avviso %d min prima per %r (%s)", minutes,
             ev.get("title"), ev.get("start"))
    start = _parse_dt(ev["start"])
    when = start.strftime("%H:%M") if start else "?"
    title = f"⏰ Tra {minutes} min · {ev['title']}"
    place = ev.get("location") or ("Online" if ev.get("online_url") else "")
    chi = ev.get("alias") or ev.get("account") or ""
    body_lines = [f"🕒 {when}  ·  👤 {chi}"]
    if place:
        body_lines.append(f"📍 {place}")
    body = "\n".join(body_lines)

    if cfg.get("windows_toast"):
        _send_windows_toast(title, body, ev.get("online_url"))
    else:
        log.info("promemoria: notifiche Windows spente in configurazione")

    tg_text = (f"⏰ <b>Riunione tra {minutes} minuti</b>\n"
               f"<b>{ev['title']}</b>\n"
               f"🕒 {when}  ·  👤 {chi}")
    if place:
        tg_text += f"\n📍 {place}"
    if ev.get("online_url"):
        tg_text += f"\n🔗 <a href=\"{ev['online_url']}\">Partecipa</a>"
    _send_telegram(cfg, tg_text)


class ReminderService(threading.Thread):
    daemon = True

    def __init__(self):
        super().__init__()
        self._stop = threading.Event()
        self.fired = _load_fired()

    def stop(self):
        self._stop.set()

    def run(self):
        # Piccolo ritardo iniziale per lasciar partire la finestra.
        self._stop.wait(5)
        while not self._stop.is_set():
            try:
                self._tick()
            except Exception:
                log.exception("promemoria: giro fallito")
            cfg = config.load_config()
            self._stop.wait(max(15, int(cfg.get("reminder_poll_seconds", 60))))

    def _tick(self):
        cfg = config.load_config()
        minutes_list = sorted(set(int(m) for m in cfg.get("reminder_minutes_before", [10])))
        if not minutes_list:
            return
        send_enabled = cfg.get("windows_toast") or cfg.get("telegram", {}).get("enabled")
        if not send_enabled:
            return

        now = datetime.now()
        horizon = now + timedelta(minutes=max(minutes_list) + 2)
        # Tutte le sorgenti, non solo Outlook: le riunioni dei calendari Google
        # e gli eventi personali devono avvisare come le altre.
        # allow_cache=False: un promemoria va mandato solo su dati freschi.
        events, errori, _ = raccolta_eventi.raccogli(
            cfg, now.isoformat(timespec="seconds"),
            horizon.isoformat(timespec="seconds"), allow_cache=False)
        if errori:
            log.info("promemoria: sorgenti con problemi: %s", errori)
        changed = False
        for ev in events:
            if ev.get("allDay"):
                continue
            start = _parse_dt(ev["start"])
            if not start or start <= now:
                continue
            for minutes in minutes_list:
                remind_at = start - timedelta(minutes=minutes)
                if now < remind_at:
                    continue  # troppo presto per questo offset
                key = f"{ev['id']}|{minutes}"
                if key in self.fired:
                    continue
                fire_reminder(cfg, ev, minutes)
                self.fired[key] = ev["start"]
                changed = True
        if changed:
            _save_fired(self.fired)


def send_test_notification(cfg):
    """Invia una notifica di prova sui canali attivi. Ritorna messaggio esito."""
    results = []
    if cfg.get("windows_toast"):
        _send_windows_toast("✅ Calendar Hub", "Notifica di prova Windows", None)
        results.append("Windows: inviata")
    tg = _send_telegram(cfg, "✅ <b>Calendar Hub</b>\nNotifica di prova Telegram")
    if tg is True:
        results.append("Telegram: inviata")
    elif isinstance(tg, str):
        results.append(f"Telegram: errore ({tg})")
    return " · ".join(results) or "Nessun canale attivo"

