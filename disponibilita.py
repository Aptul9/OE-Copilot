"""Quello che la pagina pubblica (senza accesso) può sapere degli impegni: quando, mai cosa.

La pagina pubblica è lo stesso calendario di quella con l'accesso, con gli impegni ridotti a
"Occupato". Da qui escono solo orari di inizio e fine, e le festività (date pubbliche): né un
titolo, né un luogo, né chi partecipa, né il nome di un calendario o di un'azienda. Il modulo
riceve gli eventi già letti e restituisce testi di data e ora; a quella pagina si arriva solo da
qui (app.Api.get_disponibilita).
"""
from datetime import datetime, time, timedelta


def _naive(testo):
    dt = datetime.fromisoformat(str(testo).strip().replace("Z", "+00:00"))
    return dt.astimezone().replace(tzinfo=None) if dt.tzinfo else dt


def impegni(eventi, primo_giorno, giorni, aziende=None):
    """[[inizio, fine], ...] (testo ISO, ora locale): quando si è occupati nei `giorni` giorni da
    `primo_giorno`, in ordine.

    `eventi` sono quelli di eventi.raccogli. Con `aziende` (un insieme di nomi) contano solo gli
    eventi di quelle aziende. Se si è occupati, si è occupati: gli impegni che si sovrappongono o
    si toccano diventano un blocco solo, anche a cavallo della mezzanotte. Un evento di un giorno
    intero non conta (le festività hanno la loro funzione, i "Home" dei colleghi sono rumore) e
    nemmeno uno senza durata.
    """
    da = datetime.combine(primo_giorno, time.min)
    a = da + timedelta(days=giorni)
    trovati = []
    for e in eventi:
        if e.get("allDay") or not e.get("start"):
            continue
        if aziende is not None and (e.get("alias") or e.get("account")) not in aziende:
            continue
        try:
            inizio, fine = _naive(e["start"]), _naive(e.get("end") or e["start"])
        except (ValueError, TypeError):
            continue
        if fine > inizio and fine > da and inizio < a:
            trovati.append((inizio, fine))
    uniti = []
    for inizio, fine in sorted(trovati):
        if uniti and inizio <= uniti[-1][1]:
            uniti[-1][1] = max(uniti[-1][1], fine)
        else:
            uniti.append([inizio, fine])
    return [[i.isoformat(timespec="seconds"), f.isoformat(timespec="seconds")] for i, f in uniti]


def festivita(eventi, calendario):
    """[[inizio, fine, nome], ...]: le festività, cioè gli eventi di un giorno intero del calendario
    `calendario` (config.FESTIVITA_NOME). Sono le feste di tutti, a differenza del resto."""
    trovate = set()
    for e in eventi:
        if e.get("allDay") and e.get("account") == calendario and e.get("start"):
            trovate.add((str(e["start"]), str(e.get("end") or e["start"]), str(e.get("title") or "")))
    return [list(x) for x in sorted(trovate)]
