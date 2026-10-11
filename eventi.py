"""Raccolta degli eventi da tutte le sorgenti, in un posto solo.

Le sorgenti sono tre (le caselle, i link ICS, il calendario locale) e vanno lette
insieme, non a due a due: prima questa unione stava scritta dentro l'interfaccia,
mentre il servizio promemoria leggeva per conto suo solo Outlook e gli ICS.
Risultato: gli eventi aggiunti a mano si vedevano in griglia ma non producevano
nessun promemoria. Con una funzione sola le due strade non possono più divergere.

Le caselle si leggono dall'agente remoto (agente_remoto): il pallino di ogni azienda dice
anche se sul server l'aggiornamento è fermo.
"""

import logging
import time
from datetime import datetime, time as ora_del_giorno, timedelta

import agente_remoto
import caselle
import config
import disponibilita
import ics_feed
import local_events

log = logging.getLogger(__name__)

# Le caselle non rispondono: l'agente remoto è irraggiungibile.
SORGENTE_NON_DISPONIBILE = agente_remoto.AgenteNonDisponibile


def calendari_caselle(cfg, force=False):
    """Elenco dei calendari delle caselle; solleva SORGENTE_NON_DISPONIBILE."""
    return agente_remoto.list_calendars(cfg, force=force)


def calendari_extra(cfg):
    """Le voci della barra laterale che non sono calendari delle caselle.

    Un elenco solo, usato da tutti: prima ogni punto (barra laterale, pallini,
    interruttori, filtro per azienda) sommava le sorgenti per conto suo, e una
    sorgente dimenticata in uno solo di quei punti si accendeva ma non si
    spegneva, o teneva un pallino che non si aggiornava mai.
    """
    return (ics_feed.list_feeds(cfg) + local_events.list_calendars(cfg)
            + agente_remoto.calendari_inoltrati(cfg))


def senza_doppioni(eventi):
    """Toglie lo stesso appuntamento arrivato da più fonti.

    Una riunione di un'azienda invitata sulla casella di un'altra viene accettata
    nel calendario predefinito e sta anche nel calendario Google della sua
    azienda: è lo stesso impegno e va mostrato una volta sola.

    Si tiene la copia della casella, ma se la stessa riunione arriva ANCHE dal
    calendario Google di un'azienda (l'agente marca `is_google`), si adotta la sua
    etichetta e il suo colore: la casella dove l'invito è atterrato non deve
    vincere sull'azienda a cui la riunione appartiene davvero.
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
    eventi, errori, meta = agente_remoto.fetch_all_events(
        cfg, start_iso, end_iso, allow_cache=allow_cache)
    if not config.MOSTRA_TUTTO_IL_GIORNO:
        eventi = [e for e in eventi if not e.get("allDay")]
    try:
        inizio = caselle.parse_bound(start_iso)
        fine = caselle.parse_bound(end_iso)
    except (ValueError, AttributeError) as e:
        return eventi, errori + [f"Intervallo non valido: {e}"], meta

    try:
        trovati, errori_fonte = ics_feed.fetch_events(cfg, inizio, fine)
        log.info("raccogli: calendari da link -> %d eventi", len(trovati))
        eventi += trovati
        errori += errori_fonte
    except Exception as e:
        # Una sorgente rotta non deve portarsi via le altre.
        log.exception("raccogli: calendari da link falliti")
        errori.append(f"Calendari da link: {e}")
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


def occupato(cfg, primo_giorno, giorni, gruppo=""):
    """Cosa può sapere la pagina pubblica dei `giorni` giorni da `primo_giorno`.
    -> (impegni, festività, completo), vedi disponibilita.py: solo orari, mai cosa c'è.

    Contano tutti i calendari, accesi o spenti nella barra laterale: gli interruttori sono un
    filtro della vista, e chi guarda la disponibilità non deve vedere "libero" perché ieri sera
    si è lasciato acceso un solo calendario. Con `gruppo` contano solo le aziende di quel gruppo
    (ValueError se non esiste). `completo` è falso se una sorgente non ha risposto bene: il
    chiamante lo dice, perché un calendario vuoto per un guasto sembrerebbe tutto libero.
    """
    membri = None
    if gruppo:
        membri = config.membri_gruppo(cfg, gruppo)
        if membri is None:
            raise ValueError("Gruppo non trovato")
    # copia di lavoro: senza interruttori e che non va su disco (le sorgenti salvano la config
    # quando assegnano un colore, e vi scriverebbero gli interruttori vuoti)
    cfg = dict(cfg, disabled_calendars=[], _sola_lettura=True)
    inizio = datetime.combine(primo_giorno, ora_del_giorno.min)
    eventi, errori, meta = raccogli(cfg, inizio.isoformat(),
                                    (inizio + timedelta(days=giorni)).isoformat(), allow_cache=False)
    completo = not errori and not meta.get("outlook_starting") and not meta.get("stale")
    return (disponibilita.impegni(eventi, primo_giorno, giorni, membri),
            disponibilita.festivita(eventi, config.FESTIVITA_NOME), completo)


_GRAVITA = {"ok": 0, "avviso": 1, "errore": 2}


def _stato_fonti(cfg, meta, errori=()):
    """Come è andata QUESTA lettura, azienda per azienda.

    Serve al pallino accanto a ogni calendario: prima il pallino si aggiornava
    solo quando si ricaricava l'elenco, e poteva restare verde mentre le
    riunioni non arrivavano più. Così invece racconta l'ultima lettura, che
    avviene ogni minuto. Se un'azienda ha più sorgenti, vince la peggiore.
    """
    ora = time.strftime("%H:%M")
    # L'agente remoto dice da sé com'è andata, azienda per azienda: sa se una
    # casella manca o se l'aggiornamento sul server è fermo.
    propri = meta.get("stato_aziende") or {}
    fonti = {}
    for azienda in (meta.get("aziende") or {}):
        fonti[azienda] = dict(propri.get(azienda) or {"stato": "ok", "msg": f"Letto alle {ora}"})
    try:
        altri = calendari_extra(cfg)
    except Exception:
        log.exception("stato fonti: elenco calendari non leggibile")
        return fonti
    for c in altri:
        nome = c.get("alias") or c.get("account")
        stato = c.get("stato") or "ok"
        msg = c.get("stato_msg") or ""
        prec = fonti.get(nome)
        if prec is None:
            fonti[nome] = {"stato": stato, "msg": msg}
            continue
        # Più sorgenti per la stessa azienda: si tengono tutti i messaggi e vince
        # lo stato peggiore.
        fonti[nome] = {
            "stato": stato if _GRAVITA.get(stato, 0) > _GRAVITA.get(prec["stato"], 0)
                     else prec["stato"],
            "msg": " · ".join(x for x in (prec["msg"], msg) if x),
        }
    # Ultima regola, e la più importante: se c'è un errore che nomina un'azienda, il
    # suo pallino DEVE essere rosso. Prima poteva succedere che in alto comparisse
    # "Azienda: autorizzazione non trovata" mentre il pallino di quell'azienda
    # restava verde.
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
