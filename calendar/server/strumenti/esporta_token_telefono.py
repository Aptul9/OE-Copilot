#!/usr/bin/env python3
"""Esporta in stato/tokens.json i token di aggiornamento dalle app del telefono Android virtuale.

Legge in sola lettura:
- il keystore di Android (persistent.sqlite, chiave RSA AdalKey) e la chiave avvolta adalks di Outlook
  per decifrare i RefreshToken OAuth2 (client_id 27922004-5251-4030-b22d-91ecd9a37ea4) da
  com.microsoft.identity.client.account_credential_cache.xml;
- i master token aas_et/ dei conti Google da GoogleAccountDataStore.pb e accounts_ce.db.

Non stampa mai segreti sullo standard output. Scrive il file di destinazione con permessi 0600.
"""
import base64
import hashlib
import hmac
import json
import os
import re
import shutil
import sqlite3
import struct
import sys
import tempfile
import xml.etree.ElementTree as ET
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes


def decifra_microsoft(ks_db_path, adalks_path, cache_xml_path):
    if not (os.path.exists(ks_db_path) and os.path.exists(adalks_path) and os.path.exists(cache_xml_path)):
        return {}
    cks = sqlite3.connect(ks_db_path)
    try:
        rows = cks.execute(
            "SELECT b.blob FROM blobentry b JOIN keyentry k ON b.keyentryid = k.id WHERE k.alias = 'AdalKey'"
        ).fetchall()
    finally:
        cks.close()

    with open(adalks_path, "rb") as fh:
        raw_ks = fh.read()
    if len(raw_ks) < 16:
        return {}
    _, _, meta_len = struct.unpack(">iii", raw_ks[:12])
    wrapped_key = raw_ks[12 + meta_len:]

    priv_key = None
    for (blob,) in rows:
        for offset in range(len(blob) - 32):
            if blob[offset:offset + 2] == b"\x30\x82":
                try:
                    priv_key = serialization.load_der_private_key(blob[offset:], password=None)
                    break
                except Exception:
                    pass
        if priv_key is not None:
            break
    if priv_key is None:
        return {}

    secret_key = priv_key.decrypt(wrapped_key, padding.PKCS1v15())
    hmac_key = hashlib.sha256(secret_key).digest()
    tree = ET.parse(cache_xml_path)

    def dec_voce(val):
        if not val:
            return None
        enc_ver_len = ord(val[0]) - ord("a")
        if enc_ver_len < 0 or 1 + enc_ver_len >= len(val):
            return None
        data = base64.b64decode(val[1 + enc_ver_len:])
        if len(data) < 52:
            return None
        mac_atteso = data[-32:]
        mac_vero = hmac.new(hmac_key, data[:-32], hashlib.sha256).digest()
        if not hmac.compare_digest(mac_atteso, mac_vero):
            return None
        iv = data[-48:-32]
        ct = data[4:-48]
        dec = Cipher(algorithms.AES(secret_key), modes.CBC(iv)).decryptor()
        pt_padded = dec.update(ct) + dec.finalize()
        pad_len = pt_padded[-1]
        return json.loads(pt_padded[:-pad_len].decode("utf-8"))

    accts = {}
    rts = {}
    for elem in tree.getroot():
        try:
            pt = dec_voce(elem.text or "")
        except Exception:
            continue
        if not isinstance(pt, dict):
            continue
        hid = pt.get("home_account_id")
        if not hid:
            continue
        if "authority_type" in pt and pt.get("username"):
            accts[hid] = {
                "upn": str(pt["username"]).strip().lower(),
                "tid": str(pt.get("realm") or "common").strip(),
            }
        elif pt.get("credential_type") == "RefreshToken" and pt.get("secret"):
            # Preferisce il token specifico del client rispetto a quello di famiglia
            if hid not in rts or not pt.get("family_id"):
                rts[hid] = {
                    "refresh_token": str(pt["secret"]),
                    "client_id": str(pt.get("client_id") or "27922004-5251-4030-b22d-91ecd9a37ea4"),
                }

    out = {}
    for hid, info in accts.items():
        r = rts.get(hid)
        if r and info["upn"]:
            out[info["upn"]] = {
                "provider": "microsoft",
                "client_id": r["client_id"],
                "tenant_id": info["tid"],
                "refresh_token": r["refresh_token"],
            }
    return out


def decifra_google(ce_db_path, gstore_path):
    if not (os.path.exists(ce_db_path) and os.path.exists(gstore_path)):
        return {}
    cce = sqlite3.connect(ce_db_path)
    uid_to_email = {}
    cal_tokens = {}
    try:
        for aid, name in cce.execute("SELECT _id, name FROM accounts WHERE type='com.google'"):
            email = str(name).strip().lower()
            row = cce.execute("SELECT value FROM extras WHERE accounts_id=? AND key='GoogleUserId'", (aid,)).fetchone()
            if row and row[0]:
                uid_to_email[str(row[0])] = email
            trow = cce.execute(
                "SELECT authtoken FROM authtokens WHERE accounts_id=? AND type LIKE '%calendar%'", (aid,)
            ).fetchone()
            if trow and trow[0]:
                cal_tokens[email] = str(trow[0])
    finally:
        cce.close()

    with open(gstore_path, "rb") as fh:
        raw = fh.read()
    strings = [s.decode("ascii") for s in re.findall(b"[ -~]{4,}", raw)]
    out = {}
    for idx in range(len(strings) - 1):
        if strings[idx].isdigit() and strings[idx + 1].startswith("aas_et/"):
            email = uid_to_email.get(strings[idx])
            if email:
                voce = {"provider": "google", "master_token": strings[idx + 1]}
                if email in cal_tokens:
                    voce["access_token"] = cal_tokens[email]
                out[email] = voce
    return out


def main(argv):
    base = argv[1] if len(argv) > 1 else "/opt/oe-copilot"
    dest = argv[2] if len(argv) > 2 else os.path.join(base, "stato/tokens.json")
    ks_src = os.path.join(base, "telefono/data/misc/keystore/persistent.sqlite")
    outlook_dir = os.path.join(base, "telefono/data/data/com.microsoft.office.outlook")
    adalks_src = os.path.join(outlook_dir, "app_com.microsoft.office.outlook/adalks")
    cache_xml_src = os.path.join(outlook_dir, "shared_prefs/com.microsoft.identity.client.account_credential_cache.xml")
    ce_src = os.path.join(base, "telefono/data/system_ce/0/accounts_ce.db")
    gstore_src = os.path.join(base, "telefono/data/data/com.google.android.gms/files/authaccount/shared/GoogleAccountDataStore.pb")

    esistenti = {}
    if os.path.exists(dest):
        try:
            with open(dest, "r", encoding="utf-8") as fh:
                esistenti = (json.load(fh).get("accounts") or {})
        except Exception:
            esistenti = {}

    tmp = tempfile.mkdtemp(prefix="oe-export-")
    try:
        ks_tmp = os.path.join(tmp, "ks.db")
        ce_tmp = os.path.join(tmp, "ce.db")
        for src, dst in ((ks_src, ks_tmp), (ce_src, ce_tmp)):
            if os.path.exists(src):
                shutil.copy2(src, dst)
                for suf in ("-wal", "-shm", "-journal"):
                    if os.path.exists(src + suf):
                        shutil.copy2(src + suf, dst + suf)

        ms = decifra_microsoft(ks_tmp, adalks_src, cache_xml_src)
        gg = decifra_google(ce_tmp, gstore_src)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    uniti = dict(esistenti)
    for k, v in {**ms, **gg}.items():
        prec = uniti.get(k) or {}
        uniti[k] = {**prec, **v}

    if not uniti:
        print("nessun token trovato da esportare", file=sys.stderr)
        return 1

    os.makedirs(os.path.dirname(os.path.abspath(dest)), exist_ok=True)
    tmp_out = dest + ".tmp"
    with open(tmp_out, "w", encoding="utf-8") as fh:
        json.dump({"accounts": uniti}, fh, indent=2)
        fh.write("\n")
    os.chmod(tmp_out, 0o600)
    os.replace(tmp_out, dest)
    print("esportati %d account (%d microsoft, %d google) in %s" % (len(uniti), len(ms), len(gg), dest))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
