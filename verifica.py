"""Controllo di salute dell'app: si lancia quando si vuole, dice cosa non va.

    .venv\\Scripts\\python.exe verifica.py            controlli statici + dati
    .venv\\Scripts\\python.exe verifica.py --avvio    aggiunge la prova d'avvio

Nasce dopo una serie di guasti che erano rimasti invisibili: una regola CSS
scartata dal parser per un commento malchiuso, un pulsante che ne sovrascriveva
un altro, un errore di sintassi in app.js che lasciava la finestra bianca senza
scrivere niente da nessuna parte. Ogni controllo qui sotto corrisponde a uno di
quei guasti: se torna VERDE, quel guasto non c'e'.

Esce con 1 se almeno un controllo e' ROSSO, cosi' si puo' usare anche in uno
script.
"""

import datetime
import io
import json
import os
import re
import subprocess
import sys
import time

QUI = os.path.dirname(os.path.abspath(__file__))
WEB = os.path.join(QUI, "web")

# Su console Windows l'output e' cp1252: senza questo i simboli saltano.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

esiti = []


def esito(nome, ok, dettaglio="", avviso=False):
    stato = "VERDE " if ok else ("GIALLO" if avviso else "ROSSO ")
    esiti.append((nome, ok or avviso))
    print(f"[{stato}] {nome}" + (f"\n         {dettaglio}" if dettaglio else ""))


def leggi(percorso):
    with open(percorso, "r", encoding="utf-8") as f:
        return f.read()


# ---------------------------------------------------------------- statici ----
def controlla_python():
    import py_compile
    rotti = []
    for f in sorted(os.listdir(QUI)):
        if not f.endswith((".py", ".pyw")) or f == "verifica.py":
            continue
        try:
            py_compile.compile(os.path.join(QUI, f), doraise=True)
        except py_compile.PyCompileError as e:
            rotti.append(f"{f}: {e.msg.splitlines()[-1]}")
    esito("Moduli Python compilano", not rotti, "; ".join(rotti))


def _regex_qui(out):
    """Vero se in questa posizione una "/" apre un'espressione regolare."""
    for c in reversed(out):
        if c.isspace():
            continue
        return c in "(,=:[!&|?{};+-*%~^<>"
    return True


def _senza_commenti_e_stringhe(js):
    """Toglie commenti, stringhe e regex: resta la sola struttura del codice."""
    out, i, n = [], 0, len(js)
    while i < n:
        c = js[i]
        if c == "/" and i + 1 < n and js[i + 1] == "/":
            i = js.find("\n", i)
            if i < 0:
                break
        elif c == "/" and i + 1 < n and js[i + 1] == "*":
            i = js.find("*/", i) + 2
        elif c == "/" and _regex_qui(out):
            i += 1
            while i < n:                      # corpo dell'espressione regolare
                if js[i] == "\\":
                    i += 2
                    continue
                if js[i] == "[":              # dentro [...] la / non chiude
                    while i < n and js[i] != "]":
                        i += 2 if js[i] == "\\" else 1
                if js[i] == "/":
                    i += 1
                    break
                if js[i] == chr(10):
                    break                     # non era una regex: divisione
                i += 1
            out.append("0")
        elif c in "\"'`":
            chiusura, i = c, i + 1
            while i < n:
                if js[i] == "\\":
                    i += 2
                    continue
                if js[i] == chiusura:
                    i += 1
                    break
                i += 1
            out.append('""')
        else:
            out.append(c)
            i += 1
    return "".join(out)


def controlla_js():
    js = leggi(os.path.join(WEB, "app.js"))
    corpo = _senza_commenti_e_stringhe(js)
    squilibri = [f"{a}{b}: {corpo.count(a) - corpo.count(b):+d}"
                 for a, b in ("{}", "()", "[]")
                 if corpo.count(a) != corpo.count(b)]
    esito("app.js: parentesi bilanciate", not squilibri, "; ".join(squilibri))

    esito("app.js: funzioni chiave presenti",
          all(k in js for k in ("function init(", "function setupCalendar(",
                                "function renderEventContent(", "function applicaZoom(")),
          "")

    # Ogni #id usato dal JS deve esistere davvero nell'HTML.
    html = leggi(os.path.join(WEB, "index.html"))
    presenti = set(re.findall(r'id="([^"]+)"', html))
    usati = set(re.findall(r'\$\("#([\w-]+)"\)', js)) | set(
        re.findall(r'getElementById\("([\w-]+)"\)', js))
    mancanti = sorted(usati - presenti)
    esito("Ogni #id usato dal JS esiste nell'HTML", not mancanti,
          "mancano: " + ", ".join(mancanti) if mancanti else "")

    # File referenziati dalla pagina (loghi, icone) davvero presenti.
    riferiti = set(re.findall(r'(?:src|href)="(?!https?:|data:)([^"]+)"', html))
    riferiti |= set(re.findall(r'"(logos/[\w.\-]+)"', js))
    # L'estensione personale e' facoltativa: chi non ce l'ha non deve vedere rosso.
    riferiti = {r for r in riferiti if not r.startswith("personale/")}
    assenti = sorted(r for r in riferiti if not os.path.exists(os.path.join(WEB, r)))
    esito("File referenziati dalla pagina presenti", not assenti,
          "mancano: " + ", ".join(assenti) if assenti else "")


def controlla_css():
    css = leggi(os.path.join(WEB, "style.css"))
    # Commenti malchiusi: un "*/" di troppo fa scartare al browser la regola che
    # segue, in silenzio. E' successo con l'altezza delle righe del calendario.
    resto = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    orfani = [resto[:m.start()].count("\n") + 1 for m in re.finditer(r"/\*|\*/", resto)]
    esito("CSS: commenti ben chiusi", not orfani,
          "righe sospette: " + ", ".join(map(str, orfani)) if orfani else "")

    # Stessa proprieta' dichiarata due volte per lo stesso selettore: la seconda
    # vince e di solito non era voluto (e' il caso del logo tagliato).
    blocchi = re.findall(r"(?:^|\})\s*([^{}@/][^{}]*?)\s*\{([^{}]*)\}", css, flags=re.S)
    visto, doppie = {}, []
    for selettore, corpo in blocchi:
        sel = " ".join(selettore.split())
        # "to", "from" e "45%" sono fotogrammi di @keyframes: si ripetono per
        # forza in animazioni diverse e non sono un doppione.
        if sel in ("to", "from") or re.fullmatch(r"[\d.]+%", sel):
            continue
        for prop in re.findall(r"([\w-]+)\s*:", corpo):
            chiave = (sel, prop)
            if chiave in visto:
                doppie.append(f"{sel} → {prop}")
            visto[chiave] = 1
    esito("CSS: nessuna proprietà dichiarata due volte", not doppie,
          " ; ".join(sorted(set(doppie))[:8]), avviso=bool(doppie))



def controlla_robustezza():
    """Controlli nati dai guasti veri: sono le regole che non devono tornare indietro."""
    js = leggi(os.path.join(WEB, "app.js"))
    # 1) tutte le chiamate a Python passano dal proxy che segnala gli errori.
    #    L'unica eccezione e' reportJs, che e' la via di fuga per i log.
    dirette = [n + 1 for n, r in enumerate(js.splitlines())
               if "window.pywebview.api." in r and "log_js" not in r]
    esito("Interfaccia: nessuna chiamata a Python senza rete di sicurezza",
          not dirette, "righe: " + ", ".join(map(str, dirette)) if dirette else "")

    # 2) i file di dati si scrivono in modo atomico (tmp + sostituzione): una
    #    scrittura interrotta lasciava configurazione o eventi illeggibili.
    sospette = []
    for f in sorted(os.listdir(QUI)):
        if not f.endswith(".py") or f in ("verifica.py", "config.py"):
            continue
        testo = leggi(os.path.join(QUI, f))
        for n, riga in enumerate(testo.splitlines(), 1):
            if "json.dump(" in riga and "scrivi_json" not in riga:
                sospette.append(f"{f}:{n}")
    esito("Scritture su file atomiche", not sospette, ", ".join(sospette))

    # 3) ogni modulo che cattura eccezioni deve avere un logger, se no i guasti
    #    spariscono in silenzio (outlook_com e' stato muto per mesi).
    muti = []
    for f in sorted(os.listdir(QUI)):
        if not f.endswith(".py") or f in ("verifica.py", "genera_icona.py"):
            continue
        testo = leggi(os.path.join(QUI, f))
        if "except" in testo and "getLogger" not in testo:
            muti.append(f)
    esito("Ogni modulo può scrivere nel log", not muti, ", ".join(muti))


def controlla_coerenza_stati():
    """Un errore su un'azienda DEVE corrispondere al suo pallino rosso."""
    sys.path.insert(0, QUI)
    import config                                   # noqa: E402
    import eventi as raccolta                       # noqa: E402

    meta = {"aziende": {"Azienda A": True, "Azienda B": True}}
    fonti = raccolta._stato_fonti(
        config.load_config(), meta,
        ["Azienda A: autorizzazione Google non trovata"])
    a = fonti.get("Azienda A", {})
    b = fonti.get("Azienda B", {})
    esito("Un errore rende rosso il pallino di quell'azienda",
          a.get("stato") == "errore" and b.get("stato") == "ok",
          f"A={a.get('stato')} B={b.get('stato')}")

    senza = raccolta._stato_fonti(config.load_config(), meta, [])
    esito("Senza errori nessun pallino inventato rosso",
          all(v["stato"] in ("ok", "avviso") for v in senza.values()),
          "; ".join(f"{k}={v['stato']}" for k, v in senza.items()
                    if v["stato"] not in ("ok", "avviso")))


def controlla_inoltri():
    """Aziende inoltrate e organizzatori, provati su dati finti."""
    sys.path.insert(0, QUI)
    import config                                   # noqa: E402
    import eventi as raccolta                       # noqa: E402

    # Un'azienda inoltrata di prova, senza toccare la personalizzazione vera.
    vere = config.AZIENDE_INOLTRATE
    config.AZIENDE_INOLTRATE = {"Azienda Prova": {
        "via": "nessuno@example.com", "oggetto_posta": "Copia inoltrata", "colore": None,
        "prefisso": re.compile(r"^\s*(?:Copia\s+inoltrata|INO(?:\s*-\s*Forward)?)\s*:\s*", re.I)}}
    try:
        casi = {
            "INO - Forward: Allineamento": ("Azienda Prova", "Allineamento"),
            "INO: prova": ("Azienda Prova", "prova"),
            "Copia inoltrata: Stato lavori": ("Azienda Prova", "Stato lavori"),
            "Riunione INO: stato": (None, "Riunione INO: stato"),
        }
        sbagliati = [f"{t!r} -> {config.azienda_da_titolo(t)}"
                     for t, atteso in casi.items() if config.azienda_da_titolo(t) != atteso]
    finally:
        config.AZIENDE_INOLTRATE = vere
    esito("Copie inoltrate riconosciute dal titolo (prova)", not sbagliati,
          "; ".join(sbagliati))

    # Organizzatore di un dominio aziendale -> azienda; provider generico -> nessuna.
    vecchi = (config.DOMINI_AZIENDALI, config.NOMI_PER_DOMINIO)
    config.DOMINI_AZIENDALI, config.NOMI_PER_DOMINIO = {"alfa.example"}, {"alfa.example": "Alfa"}
    try:
        trovati = (config.azienda_da_email("m.rossi@alfa.example"),
                   config.azienda_da_email("m.rossi@gmail.com"))
    finally:
        config.DOMINI_AZIENDALI, config.NOMI_PER_DOMINIO = vecchi
    esito("Organizzatori riconosciuti dal dominio (prova)", trovati == ("Alfa", None),
          "ottenuto: %r" % (trovati,))

    # Senza una voce propria un'azienda inoltrata non si potrebbe spegnere e non
    # avrebbe un pallino: le sue copie seguirebbero la casella che le ospita.
    voci = {c.get("alias") for c in raccolta.calendari_extra(config.load_config())}
    senza = [a for a in config.AZIENDE_INOLTRATE if a not in voci]
    esito("Ogni azienda inoltrata ha voce, interruttore e pallino", not senza,
          ", ".join(senza))


def _file_del_repository():
    """I file che finirebbero nel repository (tracciati o nuovi non esclusi)."""
    try:
        out = subprocess.run(["git", "ls-files", "-co", "--exclude-standard"], cwd=QUI,
                             capture_output=True, text=True, encoding="utf-8", timeout=30)
        if out.returncode == 0:
            return [os.path.join(QUI, f) for f in out.stdout.splitlines() if f.strip()]
    except (OSError, subprocess.SubprocessError):
        pass
    return None


def controlla_dati_personali():
    """Indirizzi, domini e nomi di personalizzazione.json NON devono stare nel codice.

    Il codice va condiviso; i dati di chi lo usa stanno solo nella sua
    personalizzazione (esclusa dal repository). Questo controllo li cerca uno
    per uno nei file che finirebbero nel repository.
    """
    sys.path.insert(0, QUI)
    import config                                   # noqa: E402
    try:
        with open(config.PERSONALIZZAZIONE_PATH, "r", encoding="utf-8") as f:
            p = json.load(f)
    except FileNotFoundError:
        esito("Nessun dato personale nel codice", True, "nessuna personalizzazione da confrontare")
        return
    except (OSError, ValueError) as e:
        esito("Nessun dato personale nel codice", False, "personalizzazione illeggibile: %r" % e)
        return

    cercati = set()

    def metti(*valori):
        for v in valori:
            s = str(v or "").strip().lower()
            if len(s) >= 5 and s not in GENERICI and not s.startswith(("#", "logos/")):
                cercati.add(s)

    GENERICI = {"gmail.com", "live.it", "live.com", "outlook.it", "outlook.com",
                "hotmail.it", "hotmail.com", "libero.it", "yahoo.it", "yahoo.com",
                "icloud.com", "eventi personali", "festività", "gmail", "libero",
                "outlook"}
    for sezione in ("nomi_per_indirizzo", "nomi_per_dominio", "azienda_per_cartella"):
        for k, v in (p.get(sezione) or {}).items():
            metti(k, v)
    for sezione in ("caselle_di_colleghi", "domini_aziendali", "ordine_calendari",
                    "account_nascosti"):
        metti(*(p.get(sezione) or []))
    for c in p.get("calendari_google") or []:
        metti(c.get("indirizzo"), c.get("nome"))
    for nome, d in (p.get("aziende_inoltrate") or {}).items():
        metti(nome, d.get("via"), d.get("oggetto_posta"))
    ui = p.get("interfaccia") or {}
    metti(*(ui.get("sigle") or {}).keys(), *(ui.get("loghi") or {}).keys(),
          *(ui.get("calendari_secondari") or []), *(ui.get("calendari_senza_teams") or []))
    for s in ui.get("schede_timesheet") or []:
        metti(s.get("nome"), s.get("url"), s.get("sotto"), s.get("azione"))

    file = _file_del_repository()
    if file is None:
        esito("Nessun dato personale nel codice", True, "git non disponibile: controllo saltato",
              avviso=True)
        return
    trovati = []
    for percorso in file:
        if not os.path.isfile(percorso) or percorso.lower().endswith(
                (".png", ".ico", ".jpg", ".gif", ".pyc")):
            continue
        try:
            testo = leggi(percorso).lower()
        except (OSError, UnicodeDecodeError):
            continue
        for s in cercati:
            if s in testo:
                trovati.append("%s: %s" % (os.path.relpath(percorso, QUI), s))
    esito("Nessun dato personale nel codice", not trovati,
          "; ".join(sorted(trovati)[:12]))


# ----------------------------------------------------------------- dati ------
def controlla_dati():
    sys.path.insert(0, QUI)
    import config, google_cal, outlook_com          # noqa: E402
    import app as appmod                            # noqa: E402

    cfg = config.load_config()
    esito("Configurazione leggibile", True,
          f"zoom={cfg.get('zoom_calendario')} tema={cfg.get('theme')} "
          f"calendari spenti={len(cfg.get('disabled_calendars') or [])}")

    api = appmod.Api()
    cal = api.get_calendars()
    n_out = len([c for c in cal.get("calendars", []) if not c.get("extra")])
    esito("Outlook risponde", not cal.get("error") and n_out > 0,
          cal.get("error") or f"{len(cal.get('calendars', []))} calendari")

    integr = api.get_integrazioni(verifica=True).get("integrazioni", [])
    guasti = [i for i in integr if i.get("stato") not in ("ok", None)]
    esito("Integrazioni diverse da Outlook", not guasti,
          " ; ".join(f"{i['nome']}: {i.get('stato')} {i.get('stato_msg','')}"
                     for i in integr) or "nessuna")

    oggi = datetime.datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)
    ini = oggi - datetime.timedelta(days=oggi.weekday())
    fine = ini + datetime.timedelta(days=7)
    res = api.get_events(ini.isoformat(), fine.isoformat())
    ev = res.get("events", [])
    esito("Eventi della settimana", not res.get("errors"),
          f"{len(ev)} eventi, errori: {res.get('errors') or 'nessuno'}")

    # Doppioni veri: stesso titolo e stesso inizio.
    chiavi = {}
    for e in ev:
        chiavi.setdefault((e["title"], e["start"]), []).append(e.get("alias"))
    doppi = {k: v for k, v in chiavi.items() if len(v) > 1}
    esito("Nessuna riunione doppia", not doppi,
          " ; ".join(f"{k[0][:34]} ({', '.join(v)})" for k, v in list(doppi.items())[:5]))

    # Ogni evento deve avere un'azienda e un colore: senza, in griglia esce grigio
    # e senza pallino.
    senza = [e["title"][:30] for e in ev if not e.get("alias") or not e.get("color")]
    esito("Ogni evento ha azienda e colore", not senza, ", ".join(senza[:6]))

    # L'ordine dei calendari deve nominare aziende che esistono davvero.
    # I calendari extra (festività, Google, eventi locali) non hanno "alias":
    # il nome che si vede nella barra laterale è "account".
    aliases = {c.get("alias") or c.get("account") for c in cal.get("calendars", [])}
    ignoti = [a for a in config.ORDINE_CALENDARI if a not in aliases]
    esito("L'ordine dei calendari cita solo aziende esistenti", not ignoti,
          "non trovate: " + ", ".join(ignoti) if ignoti else "", avviso=bool(ignoti))


# ---------------------------------------------------------------- avvio ------
def prova_avvio():
    """Avvia l'app davvero e verifica che l'interfaccia arrivi in fondo."""
    log = os.path.join(QUI, "debug.log")
    prima = os.path.getsize(log) if os.path.exists(log) else 0
    py = os.path.join(QUI, ".venv", "Scripts", "pythonw.exe")
    proc = subprocess.Popen([py, "app.py"], cwd=QUI)
    # WebView2 a freddo puo' metterci quasi un minuto a disegnare la pagina
    # (misurato: 54s). Con un'attesa corta il controllo dava per fallito un
    # avvio che stava solo prendendo tempo.
    inizio = time.time()
    pronta, scaduto = False, inizio + 150

    def coda_log():
        # Il log ora si appende, ma può ruotare: se il file è più corto di
        # prima, l'offset vecchio punterebbe oltre la fine e non si leggerebbe
        # nulla. Nel dubbio si rilegge da capo.
        if not os.path.exists(log):
            return ""
        da = 0 if os.path.getsize(log) < prima else prima
        with open(log, "r", encoding="utf-8", errors="replace") as f:
            f.seek(da)
            return f.read()

    while time.time() < scaduto and not pronta:
        time.sleep(2)
        nuovo = coda_log()
        if "UI: interfaccia pronta" in nuovo:
            pronta = True
        elif "AVVIO FALLITO" in nuovo:
            break
    coda = coda_log()
    errori = [r for r in coda.splitlines() if "JS:" in r or "AVVIO FALLITO" in r]
    esito("L'interfaccia si avvia", pronta and not errori,
          "\n         ".join(errori[:4]) or ("" if pronta else "nessuna conferma nel log"))
    print("         (l'app è rimasta aperta, pid %d)" % proc.pid)


if __name__ == "__main__":
    print(f"Controllo di OE Copilot — {datetime.datetime.now():%d/%m/%Y %H:%M}\n")
    controlla_python()
    controlla_js()
    controlla_css()
    controlla_robustezza()
    try:
        controlla_dati()
    except Exception as e:
        esito("Controlli sui dati", False, repr(e))
    try:
        controlla_coerenza_stati()
    except Exception as e:
        esito("Coerenza fra errori e pallini", False, repr(e))
    try:
        controlla_inoltri()
    except Exception as e:
        esito("Aziende inoltrate", False, repr(e))
    try:
        controlla_dati_personali()
    except Exception as e:
        esito("Nessun dato personale nel codice", False, repr(e))
    if "--avvio" in sys.argv:
        prova_avvio()
    rossi = [n for n, ok in esiti if not ok]
    print(f"\n{len(esiti) - len(rossi)}/{len(esiti)} controlli a posto"
          + (f" — da sistemare: {', '.join(rossi)}" if rossi else ""))
    sys.exit(1 if rossi else 0)
