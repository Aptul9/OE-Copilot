"""Raccolta degli eventi da tutte le sorgenti, in un posto solo.

Le sorgenti sono quattro (le caselle, calendari Google via CalDAV, link ICS,
calendario locale) e vanno lette insieme, non a due a due: prima questa
unione stava scritta dentro l'interfaccia, mentre il servizio promemoria leggeva
per conto suo solo Outlook e gli ICS. Risultato: le riunioni che arrivano da
un calendario Google e gli eventi aggiunti a mano si vedevano in griglia ma
non producevano nessun promemoria. Con una funzione sola le due strade non
possono piu' divergere.

Le caselle si leggono da Outlook su questo PC (outlook_com) oppure, se la
personalizzazione lo dice, da un agente remoto (agente_remoto): i due moduli
espongono le stesse funzioni e qui si sceglie quale usare.
"""

import logging
import time

import agente_remoto
import caselle
import google_cal
import ics_feed
import local_events
import outlook_com

log = logging.getLogger(__name__)

# Le caselle non rispondono: Outlook chiuso, oppure agente remoto irraggiungibile.
SORGENTE_NON_DISPONIBILE = (outlook_com.OutlookUnavailable,
                            agente_remoto.AgenteNonDisponibile)


def sorgente_caselle():
    """Il modulo da cui si leggono le caselle: agente remoto se configurato."""
    return agente_remoto if agente_remoto.attivo() else outlook_com


def calendari_caselle(cfg, force=False):
    """Elenco dei calendari delle caselle; solleva SORGENTE_NON_DISPONIBILE."""
    return sorgente_caselle().list_calendars(cfg, force=force)


def calendari_extra(cfg):
    """Le voci della barra laterale che non sono calendari delle caselle.

    Un elenco solo, usato da tutti: prima ogni punto (barra laterale, pallini,
    interruttori, filtro per azienda) sommava le sorgenti per conto suo, e una
    sorgente dimenticata in uno solo di quei punti si accendeva ma non si
    spegneva, o teneva un pallino che non si aggiornava mai.
    """
    return (ics_feed.list_feeds(cfg) + google_cal.calendari(cfg)
            + local_events.list_calendars(cfg)
            + sorgente_caselle().calendari_inoltrati(cfg))


def senza_doppioni(eventi):
    """Toglie lo stesso appuntamento arrivato da piu' fonti.

    Una riunione di un'azienda invitata sulla casella di un'altra viene accettata
    nel calendario predefinito e sta anche nel calendario Google della sua
    azienda: e' lo stesso impegno e va mostrato una volta sola.

    Si tiene la copia di Outlook (porta l'EntryID per "Apri in Outlook"), ma se
    la stessa riunione arriva ANCHE dal calendario Google di un'azienda
    (google_cal marca `is_google`), si adotta la sua etichetta e il suo colore:
    la casella dove l'invito e' atterrato non deve vincere sull'azienda a cui la
    riunione appartiene davvero.
    """
    indice = {}   # chiave -> posizione in out
    out = []
    for e in eventi:
        chiave = ((e.get("title") or "").strip().lower(), e.get("start"), e.get("end"))
        if chiave not in indice:
            indice[chiave] = len(out)
            out.append(e)
            continue
        tenuto = out[indice[chiave]]
        if tenuto.get("inoltrata_da") and not e.get("inoltrata_da"):
            # La copia di un flusso Power Automate cede il posto all'invito
            # vero, che ha il link per partecipare e l'organizzatore reale.
            out[indice[chiave]] = e
            continue
        if e.get("is_google") and not tenuto.get("is_google"):
            tenuto["alias"] = e.get("alias", tenuto.get("alias"))
            tenuto["color"] = e.get("color", tenuto.get("color"))
    return out


def raccogli(cfg, start_iso, end_iso, allow_cache=True):
    """Tutti gli eventi dell'intervallo, da ogni sorgente. -> (eventi, errori, meta)

    `start_iso`/`end_iso` sono stringhe ISO (quelle che manda il calendario).
    `allow_cache=False` vieta di rispondere con l'ultima lettura salvata: lo usa
    il servizio promemoria, che non deve avvisare per riunioni gia' spostate.
    """
    eventi, errori, meta = sorgente_caselle().fetch_all_events(
        cfg, start_iso, end_iso, allow_cache=allow_cache)
    try:
        inizio = caselle.parse_bound(start_iso)
        fine = caselle.parse_bound(end_iso)
    except (ValueError, AttributeError) as e:
        return eventi, errori + [f"Intervallo non valido: {e}"], meta

    for nome, funzione in (("Calendari da link", ics_feed.fetch_events),
                           ("Calendari Google", google_cal.fetch_events)):
        try:
            trovati, errori_fonte = funzione(cfg, inizio, fine)
            log.info("raccogli: %s -> %d eventi", nome, len(trovati))
            eventi += trovati
            errori += errori_fonte
        except Exception as e:
            # Una sorgente rotta non deve portarsi via le altre.
            log.exception("raccogli: %s ha fallito", nome)
            errori.append(f"{nome}: {e}")
    try:
        eventi += local_events.fetch_events(cfg, inizio, fine)
    except Exception as e:
        log.exception("raccogli: eventi locali falliti")
        errori.append(f"Eventi personali: {e}")

    prima = len(eventi)
    eventi = senza_doppioni(eventi)
    dopo_dedup = len(eventi)
    eventi = _solo_aziende_accese(cfg, eventi, meta.get("aziende") or {})
    if prima != len(eventi):
        log.info("raccogli: %d grezzi -> %d senza doppioni -> %d dopo il filtro aziende",
                 prima, dopo_dedup, len(eventi))
    eventi.sort(key=lambda e: e["start"] or "")
    meta["fonti"] = _stato_fonti(cfg, meta, errori)
    return eventi, errori, meta


_GRAVITA = {"ok": 0, "avviso": 1, "errore": 2}


def _stato_fonti(cfg, meta, errori=()):
    """Come e' andata QUESTA lettura, azienda per azienda.

    Serve al pallino accanto a ogni calendario: prima il pallino si aggiornava
    solo quando si ricaricava l'elenco, e poteva restare verde mentre le
    riunioni non arrivavano piu'. Cosi' invece racconta l'ultima lettura, che
    avviene ogni minuto. Se un'azienda ha piu' sorgenti, vince la peggiore.
    """
    ora = time.strftime("%H:%M")
    outlook_ok = not meta.get("outlook_starting")
    # Chi legge le caselle puo' dire da se' com'e' andata, azienda per azienda
    # (l'agente remoto lo fa: sa se una casella manca o se l'aggiornamento sul
    # server e' fermo). Outlook non lo dice: o risponde, o no.
    propri = meta.get("stato_aziende")
    fonti = {}
    for azienda in (meta.get("aziende") or {}):
        if propri is not None:
            fonti[azienda] = dict(propri.get(azienda)
                                  or {"stato": "ok", "msg": f"Letto alle {ora}"})
        else:
            fonti[azienda] = ({"stato": "ok", "msg": f"Outlook letto alle {ora}"}
                              if outlook_ok else
                              {"stato": "errore", "msg": "Outlook non risponde"})
    try:
        altri = calendari_extra(cfg)
    except Exception:
        log.exception("stato fonti: elenco calendari non leggibile")
        return fonti
    for c in altri:
        nome = c.get("alias") or c.get("account")
        stato = c.get("stato") or "ok"
        msg = c.get("stato_msg") or ""
        if c.get("is_inoltro") and propri is None:
            # Le copie si leggono dal calendario di un'altra casella di
            # Outlook: se Outlook non risponde non arrivano nemmeno loro.
            # (Con l'agente remoto il pallino segue gia' la casella che le ospita.)
            if not outlook_ok:
                stato, msg = "errore", "Outlook non risponde"
            else:
                msg = f"Outlook letto alle {ora} · {msg}"
        prec = fonti.get(nome)
        if prec is None:
            fonti[nome] = {"stato": stato, "msg": msg}
            continue
        # Piu' sorgenti per la stessa azienda (es. su Google e in Outlook):
        # si tengono tutti i messaggi e vince lo stato peggiore.
        fonti[nome] = {
            "stato": stato if _GRAVITA.get(stato, 0) > _GRAVITA.get(prec["stato"], 0)
                     else prec["stato"],
            "msg": " · ".join(x for x in (prec["msg"], msg) if x),
        }
    # Ultima regola, e la piu' importante: se c'e' un errore che nomina
    # un'azienda, il suo pallino DEVE essere rosso. Prima poteva succedere che
    # in alto comparisse "Azienda: autorizzazione non trovata" mentre il
    # pallino di quell'azienda restava verde.
    for testo in errori or ():
        nome, _, dettaglio = str(testo).partition(":")
        nome = nome.strip()
        if nome in fonti:
            fonti[nome] = {"stato": "errore", "msg": (dettaglio.strip() or nome)}
    return fonti


def _solo_aziende_accese(cfg, eventi, aziende_outlook):
    """Toglie gli eventi delle aziende spente in "I miei calendari".

    Il filtro guarda l'AZIENDA mostrata, non la cartella che ospita l'evento.
    Outlook archivia gli inviti indirizzati a un account IMAP (che non ha un
    calendario sul server) nel calendario predefinito, che e' di un'altra
    azienda: filtrando per cartella, spegnere la loro azienda le lasciava
    visibili e spegnere l'altra le faceva sparire. Sono riunioni della loro
    azienda e devono seguire quell'interruttore.
    """
    acceso = dict(aziende_outlook)
    for c in calendari_extra(cfg):
        nome = c.get("alias") or c.get("account")
        acceso[nome] = acceso.get(nome, False) or bool(c.get("enabled"))
    spente = {nome for nome, on in acceso.items() if not on}
    if not spente:
        return eventi
    return [e for e in eventi if (e.get("alias") or e.get("account")) not in spente]
