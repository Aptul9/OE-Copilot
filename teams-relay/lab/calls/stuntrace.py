"""Every ICE/STUN message and DTLS record of a capture in time order, through TURN (ChannelData mapped to its peer by
the ChannelBind requests, Send/Data indications by XOR-PEER-ADDRESS) or direct: time, direction, peer, how, what.
argv[2] (optional): only lines whose peer starts with it."""
import struct, sys

LOCAL = "172."
NAMES = {0x0006: "USERNAME", 0x0008: "MI", 0x8028: "FP", 0x0024: "PRIORITY", 0x0025: "USE-CANDIDATE", 0x8029: "CONTROLLED",
         0x802A: "CONTROLLING", 0x8070: "MS-IMPL-VER", 0x0020: "XOR-MAPPED", 0x8054: "CAND-ID", 0xC057: "NET-COST",
         0x0009: "ERROR", 0x8095: "MS-MUX-ID", 0x8036: "x8036", 0x8037: "x8037", 0x000C: "CHANNEL", 0x0012: "XOR-PEER",
         0x0013: "DATA", 0x8022: "SOFTWARE", 0x0016: "XOR-RELAYED", 0x000D: "LIFETIME", 0x0019: "REQ-TRANSPORT",
         0x0014: "REALM", 0x0015: "NONCE", 0x8008: "MS-VERSION", 0x8050: "MS-SEQ", 0x8055: "MS-SVC-QUALITY"}
CLASSES = {0: "req", 1: "ind", 2: "ok", 3: "err"}
METHODS = {1: "Binding", 3: "Allocate", 4: "Refresh", 6: "Send", 7: "Data", 8: "CreatePerm", 9: "ChanBind"}
HS = {1: "ClientHello", 2: "ServerHello", 3: "HelloVerifyRequest", 11: "Certificate", 12: "ServerKeyExchange", 13: "CertificateRequest",
      14: "ServerHelloDone", 15: "CertificateVerify", 16: "ClientKeyExchange", 20: "Finished"}


def xaddr(v, tid=None):
    if len(v) < 8:
        return "?"
    port = struct.unpack("!H", v[2:4])[0] ^ 0x2112
    if v[1] == 1:
        return ".".join(str(b ^ m) for b, m in zip(v[4:8], b"\x21\x12\xa4\x42")) + f":{port}"
    return f"v6:{port}"


def stun(b):
    if len(b) < 20 or b[0] >= 2 or b[4:8] != b"\x21\x12\xa4\x42":
        return None
    t = struct.unpack("!H", b[:2])[0]
    method = (t & 0x000F) | ((t & 0x00E0) >> 1) | ((t & 0x3E00) >> 2)
    cls = ((t & 0x0100) >> 7) | ((t & 0x0010) >> 4)
    attrs, i = [], 20
    while i + 4 <= len(b):
        a, l = struct.unpack("!HH", b[i:i + 4])
        attrs.append((a, b[i + 4:i + 4 + l]))
        i += 4 + l + (4 - l % 4) % 4
    return method, cls, b[8:20], attrs


def show(attrs):
    out = []
    for a, v in attrs:
        n = NAMES.get(a, hex(a))
        if n in ("MI", "FP", "DATA", "REALM", "NONCE", "SOFTWARE"):
            continue
        if n == "USERNAME":
            out.append(f"U={v.decode(errors='replace')}")
        elif n == "PRIORITY":
            out.append(f"PRI={struct.unpack('!I', v)[0]}")
        elif n in ("XOR-MAPPED", "XOR-PEER", "XOR-RELAYED"):
            out.append(f"{n}={xaddr(v)}")
        elif n == "ERROR" and len(v) >= 4:
            out.append(f"ERR={v[2] * 100 + v[3]} {v[4:].decode(errors='replace')}")
        elif n == "CHANNEL":
            out.append(f"CH={struct.unpack('!H', v[:2])[0]:#x}")
        elif n in ("USE-CANDIDATE",):
            out.append(n)
        else:
            out.append(f"{n}={v.hex()}")
    return " ".join(out)


def dtls(p):
    out, i = [], 0
    while i + 13 <= len(p):
        ct = p[i]
        if ct not in (20, 21, 22, 23):
            return None
        ln = struct.unpack("!H", p[i + 11:i + 13])[0]
        body = p[i + 13:i + 13 + ln]
        epoch = struct.unpack("!H", p[i + 3:i + 5])[0]
        if ct == 22 and epoch == 0 and body:
            out.append(HS.get(body[0], f"hs{body[0]}"))
        elif ct == 21 and epoch == 0:
            out.append(f"Alert{tuple(body[:2])}")
        else:
            out.append({20: "CCS", 21: "Alert", 22: "HS", 23: "AppData"}[ct] + f"(e{epoch})")
        i += 13 + ln
    return out


def what(p):
    s = stun(p)
    if s:
        m, c, tid, attrs = s
        return f"{METHODS.get(m, hex(m))}-{CLASSES[c]} t={tid.hex()[:8]} {show(attrs)}"
    d = dtls(p)
    if d:
        return "DTLS " + " ".join(d)
    if p and 128 <= p[0] <= 191:
        return f"RTP/RTCP pt={p[1] & 0x7f}"
    return f"? {p[:4].hex()}"


channels = {}
only = sys.argv[2] if len(sys.argv) > 2 else None
f = open(sys.argv[1], "rb")
f.read(24)
t0 = None
while True:
    h = f.read(16)
    if len(h) < 16:
        break
    ts, us, cap, _ = struct.unpack("<IIII", h)
    eth = f.read(cap)[14:]
    if len(eth) < 20 or eth[9] != 17:
        continue
    ihl = (eth[0] & 15) * 4
    src, dst = ".".join(map(str, eth[12:16])), ".".join(map(str, eth[16:20]))
    sp, dp = struct.unpack("!HH", eth[ihl:ihl + 4])
    if 53 in (sp, dp):
        continue
    udp = eth[ihl + 8:]
    t = ts + us / 1e6
    t0 = t0 or t
    out = src.startswith(LOCAL)
    d = "out" if out else "in "
    server = f"{dst}:{dp}" if out else f"{src}:{sp}"
    if len(udp) >= 4 and 0x40 <= udp[0] <= 0x7F:
        ch = struct.unpack("!H", udp[:2])[0]
        peer = channels.get((server, ch), f"ch{ch:#x}")
        line = f"{t - t0:8.3f} {d} {peer:22} chan  {what(udp[4:4 + struct.unpack('!H', udp[2:4])[0]])}"
    else:
        s = stun(udp)
        if s and s[0] in (6, 7) and s[1] == 1:
            a = dict(s[3])
            peer = xaddr(a.get(0x0012, b""))
            line = f"{t - t0:8.3f} {d} {peer:22} ind   {what(a.get(0x0013, b''))}"
        elif s and s[0] in (3, 4, 8, 9):
            a = dict(s[3])
            if s[0] == 9 and s[1] == 0 and 0x000C in a:
                channels[(server, struct.unpack("!H", a[0x000C][:2])[0])] = xaddr(a.get(0x0012, b""))
            line = f"{t - t0:8.3f} {d} {server:22} turn  {what(udp)}"
        else:
            line = f"{t - t0:8.3f} {d} {server:22} direct {what(udp)}"
    if only and only not in line:
        continue
    print(line)
