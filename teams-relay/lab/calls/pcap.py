"""DTLS records inside the TURN traffic of a capture: ChannelData and Send/Data indications, per direction."""
import struct, sys

LOCAL = "172."
HS = {1: "ClientHello", 2: "ServerHello", 3: "HelloVerifyRequest", 11: "Certificate", 12: "ServerKeyExchange", 13: "CertificateRequest", 14: "ServerHelloDone", 15: "CertificateVerify", 16: "ClientKeyExchange", 20: "Finished"}
CT = {20: "ChangeCipherSpec", 21: "Alert", 22: "Handshake", 23: "AppData"}

def ip(b):
    return ".".join(str(x) for x in b)

def dtls(payload):
    out, i = [], 0
    while i + 13 <= len(payload):
        ct = payload[i]
        if ct not in CT:
            return None
        ln = struct.unpack("!H", payload[i + 11:i + 13])[0]
        body = payload[i + 13:i + 13 + ln]
        epoch = struct.unpack("!H", payload[i + 3:i + 5])[0]
        if ct == 22 and epoch == 0 and body:
            out.append(HS.get(body[0], f"hs{body[0]}"))
        elif ct == 21 and epoch == 0 and len(body) >= 2:
            out.append(f"Alert({body[0]},{body[1]})")
        else:
            out.append(f"{CT[ct]}(e{epoch})")
        i += 13 + ln
    return out

def inner(udp):
    if len(udp) >= 4 and 0x40 <= udp[0] <= 0x7F:
        ln = struct.unpack("!H", udp[2:4])[0]
        return "chan", udp[4:4 + ln]
    if len(udp) >= 20 and udp[0] < 2:
        mtype = struct.unpack("!H", udp[:2])[0]
        if mtype in (0x0016, 0x0017):
            i = 20
            while i + 4 <= len(udp):
                t, l = struct.unpack("!HH", udp[i:i + 4])
                if t == 0x0013:
                    return "send" if mtype == 0x16 else "data", udp[i + 4:i + 4 + l]
                i += 4 + l + (4 - l % 4) % 4
        return f"stun{mtype:#06x}", b""
    return "raw", udp

if len(sys.argv) > 2 and sys.argv[2] == "ssrc":
    # SRTP keeps the RTP header in clear: SSRC and payload type of every RTP packet, per direction
    from collections import Counter
    seen = Counter()
    f = open(sys.argv[1], "rb")
    f.read(24)
    while True:
        h = f.read(16)
        if len(h) < 16:
            break
        cap = struct.unpack("<IIII", h)[2]
        eth = f.read(cap)[14:]
        if len(eth) < 20 or eth[9] != 17:
            continue
        src = ip(eth[12:16])
        ihl = (eth[0] & 15) * 4
        udp = eth[ihl + 8:]
        kind, payload = inner(udp)
        p = payload if kind in ("chan", "send", "data", "raw") else b""
        if len(p) >= 12 and 128 <= p[0] <= 191:
            pt = p[1] & 0x7F
            if 72 <= pt <= 76:
                seen[("out" if src.startswith(LOCAL) else "in ", kind, "rtcp", p[1])] += 1
            else:
                seen[("out" if src.startswith(LOCAL) else "in ", kind, f"pt{pt}", struct.unpack("!I", p[8:12])[0])] += 1
    for k, v in sorted(seen.items(), key=lambda x: -x[1])[:20]:
        print(v, *k)
    sys.exit(0)

f = open(sys.argv[1], "rb")
f.read(24)
t0 = None
while True:
    h = f.read(16)
    if len(h) < 16:
        break
    ts, us, cap, _ = struct.unpack("<IIII", h)
    pkt = f.read(cap)
    eth = pkt[14:]
    if len(eth) < 20 or eth[9] != 17:
        continue
    ihl = (eth[0] & 15) * 4
    src, dst = ip(eth[12:16]), ip(eth[16:20])
    sp, dp = struct.unpack("!HH", eth[ihl:ihl + 4])
    if 53 in (sp, dp) or 5353 in (sp, dp):
        continue
    udp = eth[ihl + 8:]
    t = ts + us / 1e6
    t0 = t0 or t
    kind, payload = inner(udp)
    d = dtls(payload) if payload else None
    direction = "out" if src.startswith(LOCAL) else "in "
    peer = f"{dst}:{dp}" if direction == "out" else f"{src}:{sp}"
    if d:
        print(f"{t - t0:7.3f} {direction} {peer} {kind} {len(udp)}B DTLS {' '.join(d)}")
    elif kind == "raw" and payload and 128 <= payload[0] <= 191:
        print(f"{t - t0:7.3f} {direction} {peer} raw {len(udp)}B RTP/RTCP")
    elif kind in ("chan", "send", "data") and payload and 128 <= payload[0] <= 191:
        print(f"{t - t0:7.3f} {direction} {peer} {kind} {len(udp)}B RTP/RTCP")
