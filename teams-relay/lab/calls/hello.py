"""The first DTLS ClientHello of a capture (inside ChannelData, Send/Data indication or direct), decoded: version,
cipher suites, extensions with their values."""
import struct, sys

EXT = {0: "server_name", 10: "supported_groups", 11: "ec_point_formats", 13: "signature_algorithms", 14: "use_srtp",
       23: "extended_master_secret", 65281: "renegotiation_info", 35: "session_ticket", 16: "alpn", 43: "supported_versions"}


def payloads(path):
    f = open(path, "rb")
    f.read(24)
    while True:
        h = f.read(16)
        if len(h) < 16:
            return
        cap = struct.unpack("<IIII", h)[2]
        eth = f.read(cap)[14:]
        if len(eth) < 20 or eth[9] != 17:
            continue
        ihl = (eth[0] & 15) * 4
        src = ".".join(map(str, eth[12:16]))
        udp = eth[ihl + 8:]
        if len(udp) >= 4 and 0x40 <= udp[0] <= 0x7F:
            yield src, udp[4:4 + struct.unpack("!H", udp[2:4])[0]]
        elif len(udp) >= 20 and udp[0] < 2 and struct.unpack("!H", udp[:2])[0] in (0x16, 0x17):
            i = 20
            while i + 4 <= len(udp):
                t, l = struct.unpack("!HH", udp[i:i + 4])
                if t == 0x13:
                    yield src, udp[i + 4:i + 4 + l]
                i += 4 + l + (4 - l % 4) % 4
        else:
            yield src, udp


want = sys.argv[2] if len(sys.argv) > 2 else "out"
for src, p in payloads(sys.argv[1]):
    out = src.startswith("172.")
    if (want == "out") != out:
        continue
    if len(p) < 25 or p[0] != 22 or p[13] != 1:
        continue
    print("record version", p[1:3].hex(), "len", len(p))
    b = p[13:]
    body = b[12:]
    print("hello version", body[0:2].hex())
    i = 2 + 32
    sid = body[i]
    i += 1 + sid
    cookie = body[i]
    print("cookie len", cookie)
    i += 1 + cookie
    n = struct.unpack("!H", body[i:i + 2])[0]
    suites = [body[i + 2 + k:i + 4 + k].hex() for k in range(0, n, 2)]
    print("suites", suites)
    i += 2 + n
    c = body[i]
    i += 1 + c
    el = struct.unpack("!H", body[i:i + 2])[0]
    i += 2
    end = i + el
    while i + 4 <= end:
        t, l = struct.unpack("!HH", body[i:i + 4])
        v = body[i + 4:i + 4 + l]
        print(" ext", t, EXT.get(t, "?"), v.hex(), v[5:].decode(errors="replace") if t == 0 else "")
        i += 4 + l
    break
