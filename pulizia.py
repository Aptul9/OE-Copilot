"""Riunioni finite nel calendario sbagliato: le trova e (se richiesto) le sposta.

Perche' succede: un account configurato in Outlook come IMAP non ha un
calendario sul server. Quando arriva un invito indirizzato a quell'indirizzo,
Outlook non ha dove metterlo e lo archivia nel calendario PREDEFINITO del
profilo, che e' di un'altra azienda. Risultato: riunioni di un'azienda che
dentro Outlook stanno sotto un'altra.

Uso da riga di comando ("Azienda" e' il nome mostrato nell'app):

    .venv\\Scripts\\python.exe pulizia.py "Azienda"             elenco, non tocca nulla
    .venv\\Scripts\\python.exe pulizia.py "Azienda" --applica   sposta davvero

Da rilanciare ogni tanto: quando l'organizzatore manda un aggiornamento, Outlook
lo elabora sul calendario predefinito e ricrea li' la voce. Ogni spostamento
finisce in un registro JSON (con gli identificativi prima/dopo), quindi si puo'
sempre tornare indietro.
"""

import collections
import json
import logging
import os
import sys
import time
from datetime import datetime, timedelta

import pythoncom

import config
import outlook_com

log = logging.getLogger(__name__)

STATO = {0: "appuntamento", 1: "la organizzi tu", 3: "invito ricevuto",
         5: "invito annullato", 7: "riunione annullata"}

# Spostando fra archivi diversi Outlook antepone "Copia: " al titolo. Va tolto:
# col titolo cambiato l'evento non si riconosce piu' come lo stesso letto da
# Google e comparirebbe due volte in griglia.
PREFISSI_COPIA = ("Copia: ", "Copy: ", "Copia di ")


def _azienda_evento(cfg, item, label, account, mie, alias_casella):
    """Azienda dell'evento, con le stesse regole dell'app (cartella dedicata,
    organizzatore, indirizzo invitato). Ritorna (azienda, azienda_della_cartella)."""
    della_cartella = (config.azienda_da_cartella(label)
                      or alias_casella.get(account, account))
    azienda = config.azienda_da_cartella(label)
    if not azienda:
        azienda = config.azienda_da_email(outlook_com._organizer_smtp(item))
    if not azienda:
        try:
            eid = item.EntryID
        except pythoncom.com_error:
            eid = ""
        azienda = config.azienda_da_email(
            outlook_com._casella_invitata(item, mie, eid))
    return (azienda or della_cartella), della_cartella


def elenco(cfg, da_giorni=180, a_giorni=365):
    """Elementi che stanno in un calendario di un'altra azienda. SOLA LETTURA."""
    oggi = datetime.now()
    lo, hi = oggi - timedelta(days=da_giorni), oggi + timedelta(days=a_giorni)
    fuori_posto = []
    with outlook_com._com_lock, outlook_com._Session(cfg.get("outlook_profile")) as ns:
        mie = outlook_com._mie_caselle(ns)
        alias_casella = {}
        for folder, account, cal_id, label in outlook_com._iter_calendars(ns):
            if account not in alias_casella:
                alias_casella.update(config.alias_per_account(cfg, [account]))
            try:
                items = outlook_com._restricted_items(folder, lo, hi)
            except pythoncom.com_error:
                continue
            for n, item in enumerate(items):
                if n >= outlook_com.MAX_ITEMS_PER_FOLDER:
                    break
                try:
                    azienda, della_cartella = _azienda_evento(
                        cfg, item, label, account, mie, alias_casella)
                    if azienda == della_cartella:
                        continue
                    fuori_posto.append({
                        "entry_id": item.EntryID,
                        "titolo": item.Subject or "(senza titolo)",
                        "inizio": str(outlook_com._to_naive_local(item.Start)),
                        "dove_sta": label,
                        "azienda": azienda,
                        "tipo": STATO.get(getattr(item, "MeetingStatus", 0), "?"),
                    })
                except pythoncom.com_error:
                    continue
    fuori_posto.sort(key=lambda e: e["inizio"])
    return fuori_posto


def _togli_prefisso_copia(item):
    try:
        titolo = item.Subject or ""
        for p in PREFISSI_COPIA:
            if titolo.startswith(p):
                item.Subject = titolo[len(p):]
                item.Save()
                return True
    except pythoncom.com_error:
        log.warning("pulizia: titolo non ripulito", exc_info=True)
    return False


def sposta(cfg, entry_ids, dest_entry_id, dest_store_id=None, registro=None):
    """Sposta gli elementi indicati nel calendario di destinazione.

    Spostare cambia l'EntryID: senza annotarlo non si potrebbe piu' tornare
    indietro, quindi ogni spostamento va in un registro JSON con provenienza e
    identificativi prima/dopo.

    Ritorna (spostati, errori, voci_registro).
    """
    spostati, errori, voci = 0, [], []
    with outlook_com._com_lock, outlook_com._Session(cfg.get("outlook_profile")) as ns:
        try:
            destinazione = (ns.GetFolderFromID(dest_entry_id, dest_store_id)
                            if dest_store_id else ns.GetFolderFromID(dest_entry_id))
        except pythoncom.com_error as e:
            return 0, ["Calendario di destinazione non raggiungibile: %s" % e], []
        for eid in entry_ids:
            titolo = "?"
            try:
                item = ns.GetItemFromID(eid)
                titolo = item.Subject or "?"
                partenza = item.Parent.FolderPath
                nuovo_item = item.Move(destinazione)
                _togli_prefisso_copia(nuovo_item)
                voci.append({"titolo": titolo, "da": partenza,
                             "a": destinazione.FolderPath,
                             "entry_id_prima": eid,
                             "entry_id_dopo": getattr(nuovo_item, "EntryID", "")})
                spostati += 1
                log.info("pulizia: spostato %r da %s", titolo, partenza)
            except pythoncom.com_error as e:
                errori.append("%s: %s" % (titolo, e))
                log.warning("pulizia: non spostato %r (%s)", titolo, e)
    percorso = registro or os.path.join(
        config.APP_DIR, "pulizia_%s.json" % time.strftime("%Y%m%d-%H%M%S"))
    try:
        # Registro di cio' che e' stato spostato: e' l'unico modo per tornare
        # indietro, quindi si scrive in modo atomico come la configurazione.
        config.scrivi_json(percorso, {"destinazione": dest_entry_id, "voci": voci})
    except OSError:
        log.warning("pulizia: registro non scritto", exc_info=True)
    return spostati, errori, voci


def destinazione_azienda(cfg, azienda):
    """Calendario in cui devono stare le riunioni di quell'azienda."""
    with outlook_com._com_lock, outlook_com._Session(cfg.get("outlook_profile")) as ns:
        alias = {}
        for e in outlook_com._discover(ns):
            if e["account"] not in alias:
                alias.update(config.alias_per_account(cfg, [e["account"]]))
            nome = config.azienda_da_cartella(e["label"]) or alias.get(e["account"])
            if nome == azienda and "compleanni" not in e["label"].lower():
                return e
    return None


def main(azienda, applica=False):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    cfg = config.load_config()
    fuori = [e for e in elenco(cfg) if e["azienda"] == azienda]
    per_id = collections.OrderedDict()
    for e in fuori:
        per_id.setdefault(e["entry_id"], []).append(e)

    print("Riunioni di %s in un calendario di un'altra azienda: %d occorrenze, "
          "%d elementi (una serie conta una volta)"
          % (azienda, len(fuori), len(per_id)))
    print("")
    for occ in per_id.values():
        nota = "serie x%d" % len(occ) if len(occ) > 1 else occ[0]["tipo"]
        print("  %s | %-36s | %-16s | %s"
              % (occ[0]["inizio"][:16], occ[0]["dove_sta"][:34], nota,
                 occ[0]["titolo"][:44]))
    if not per_id:
        print("  niente fuori posto.")
        return 0

    dest = destinazione_azienda(cfg, azienda)
    if not dest:
        print("")
        print("Nessun calendario per %s: non saprei dove metterle." % azienda)
        return 1
    print("")
    print("Destinazione: %s" % dest["label"])
    if not applica:
        print("Prova a vuoto: non ho spostato niente.")
        print("Per farlo davvero:  .venv\\Scripts\\python.exe pulizia.py --applica")
        return 0

    spostati, errori, _ = sposta(cfg, list(per_id), dest["entry_id"], dest["store_id"])
    print("")
    print("Spostati %d elementi su %d." % (spostati, len(per_id)))
    for e in errori:
        print("  non spostato:", e[:120])
    # L'elenco dei calendari va riletto: quello in cache potrebbe non contenere
    # ancora la destinazione, e gli eventi spostati sembrerebbero spariti.
    outlook_com.list_calendars(cfg, force=True)
    return 0


if __name__ == "__main__":
    nomi = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not nomi:
        print('Uso: pulizia.py "Azienda" [--applica]')
        raise SystemExit(2)
    raise SystemExit(main(nomi[0], applica="--applica" in sys.argv))
