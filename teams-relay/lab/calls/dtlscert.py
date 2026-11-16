"""The DTLS handshake of a capture in detail: ClientHello extensions, CertificateRequest, the peer's certificate (DER
written out) and the signature algorithm of its CertificateVerify."""
import struct, sys, subprocess

SIG = {0x0401: "rsa_pkcs1_sha256", 0x0501: "rsa_pkcs1_sha384", 0x0601: "rsa_pkcs1_sha512", 0x0201: "rsa_pkcs1_sha1", 0x0203: "ecdsa_sha1",
       0x0403: "ecdsa_secp256r1_sha256", 0x0503: "ecdsa_secp384r1_sha384", 0x0603: "ecdsa_secp521r1_sha512", 0x0804: "rsa_pss_rsae_sha256",
       0x0805: "rsa_pss_rsae_sha384", 0x0806: "rsa_pss_rsae_sha512", 0x0807: "ed25519", 0x0809: "rsa_pss_pss_sha256"}

def records(path):
    f = open(path, "rb"); f.read(24)
    while True:
        h = f.read(16)
        if len(h) < 16:
            return
        cap = struct.unpack("<IIII", h)[2]
        eth = f.read(cap)[14:]
        if len(eth) < 20 or eth[9] != 17:
            continue
        src = ".".join(map(str, eth[12:16]))
        udp = eth[(eth[0] & 15) * 4 + 8:]
        if 0x40 <= udp[0] <= 0x7F:
            udp = udp[4:]
        i = 0
        while i + 13 <= len(udp) and 20 <= udp[i] <= 23:
            ln = struct.unpack("!H", udp[i + 11:i + 13])[0]
            yield ("out" if src.startswith("172.") else "in"), udp[i], struct.unpack("!H", udp[i + 3:i + 5])[0], udp[i + 13:i + 13 + ln]
            i += 13 + ln

def exts(body, i):
    n = struct.unpack("!H", body[i:i + 2])[0]; i += 2; end = i + n; out = {}
    while i < end:
        t, l = struct.unpack("!HH", body[i:i + 4]); out[t] = body[i + 4:i + 4 + l]; i += 4 + l
    return out

seen = set()
for d, ct, epoch, rec in records(sys.argv[1]):
    if ct != 22 or epoch != 0:
        continue
    j = 0
    while j + 12 <= len(rec):
        t = rec[j]; ln = int.from_bytes(rec[j + 1:j + 4], "big"); flen = int.from_bytes(rec[j + 9:j + 12], "big")
        body = rec[j + 12:j + 12 + flen]
        j += 12 + flen
        key = (d, t)
        if key in seen:
            continue
        seen.add(key)
        if t == 1:
            i = 2 + 32; i += 1 + body[i]; i += 1 + body[i]
            n = struct.unpack("!H", body[i:i + 2])[0]
            print(d, "ClientHello suites", [body[i + 2 + k:i + 4 + k].hex() for k in range(0, n, 2)])
            i += 2 + n; i += 1 + body[i]
            e = exts(body, i)
            print(d, "  exts", sorted(e))
            if 13 in e:
                s = e[13]; print(d, "  sig_algs", [SIG.get(struct.unpack("!H", s[2 + k:4 + k])[0], s[2 + k:4 + k].hex()) for k in range(0, len(s) - 2, 2)])
            if 10 in e:
                print(d, "  groups", e[10][2:].hex())
            if 14 in e:
                print(d, "  use_srtp", e[14].hex())
        elif t == 2:
            print(d, "ServerHello suite", body[2 + 32 + 1 + body[34]:][:2].hex(), "exts", sorted(exts(body, 2 + 32 + 1 + body[34] + 3)))
        elif t == 13:
            k = 1 + body[0]
            n = struct.unpack("!H", body[k:k + 2])[0]
            print(d, "CertificateRequest sig_algs", [SIG.get(struct.unpack("!H", body[k + 2 + m:k + 4 + m])[0], body[k + 2 + m:k + 4 + m].hex()) for m in range(0, n, 2)])
        elif t == 11:
            first = int.from_bytes(body[3:6], "big")
            der = body[6:6 + first]
            name = f"/tmp/cert-{d}.der"; open(name, "wb").write(der)
            print(d, "Certificate", len(der), "bytes, fragment", flen, "of", ln)
            r = subprocess.run(["openssl", "x509", "-inform", "DER", "-in", name, "-noout", "-text"], capture_output=True, text=True)
            for line in r.stdout.splitlines():
                if any(k in line for k in ("Signature Algorithm", "Public Key Algorithm", "Public-Key", "ASN1 OID", "NIST CURVE", "Not Before", "Not After", "Issuer", "Subject:", "Version")):
                    print(d, "   ", line.strip())
            if r.returncode:
                print(d, "   openssl:", r.stderr.strip()[:300])
            fp = subprocess.run(["openssl", "x509", "-inform", "DER", "-in", name, "-noout", "-fingerprint", "-sha256"], capture_output=True, text=True)
            print(d, "   ", fp.stdout.strip())
        elif t == 15:
            print(d, "CertificateVerify", SIG.get(struct.unpack("!H", body[:2])[0], body[:2].hex()), "sig len", struct.unpack("!H", body[2:4])[0])
        elif t == 16:
            print(d, "ClientKeyExchange len", len(body))
