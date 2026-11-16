"""ICE checks inside the TURN traffic of a capture: per direction and peer, requests/responses and their attributes."""
import struct, sys
from collections import Counter

NAMES = {0x0006: "USERNAME", 0x0008: "MI", 0x8028: "FP", 0x0024: "PRIORITY", 0x0025: "USE-CANDIDATE", 0x8029: "CONTROLLED", 0x802A: "CONTROLLING", 0x8070: "MS-VERSION", 0x0020: "XOR-MAPPED", 0x8054: "CAND-ID", 0x8036: "0x8036", 0x8037: "0x8037", 0x8095: "0x8095", 0x8055: "MS-NET-ID", 0xC057: "NET-COST"}
seen = Counter()

def stun(b):
    if len(b) < 20 or b[0] >= 2 or b[4:8] != b"\x21\x12\xa4\x42":
        return None
    t = struct.unpack("!H", b[:2])[0]
    attrs, i = [], 20
    while i + 4 <= len(b):
        a, l = struct.unpack("!HH", b[i:i + 4])
        attrs.append(NAMES.get(a, hex(a)))
        i += 4 + l + (4 - l % 4) % 4
    return t, attrs

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
    src = ".".join(map(str, eth[12:16]))
    udp = eth[(eth[0] & 15) * 4 + 8:]
    out = src.startswith("172.")
    dst = ".".join(map(str, eth[16:20]))
    sp, dp = struct.unpack("!HH", eth[(eth[0] & 15) * 4:(eth[0] & 15) * 4 + 4])
    peer = (dst, dp) if out else (src, sp)
    inner = None
    if peer[1] != 3478 and peer[1] != 53:
        s = stun(udp)
        kind = "non-stun" if not s else {0x0001: "req", 0x0101: "ok", 0x0111: "err"}.get(s[0], hex(s[0]))
        attrs = "" if not s else " ".join(x for x in s[1] if x not in ("MI", "FP", "USERNAME"))
        seen[("out" if out else "in ", "direct", f"{peer[0]}:{peer[1]}", kind, attrs)] += 1
        continue
    if 0x40 <= udp[0] <= 0x7F:
        inner = udp[4:4 + struct.unpack("!H", udp[2:4])[0]]
        how = "chan"
    else:
        s = stun(udp)
        if s and s[0] in (0x0016, 0x0017):
            i = 20
            while i + 4 <= len(udp):
                a, l = struct.unpack("!HH", udp[i:i + 4])
                if a == 0x0013:
                    inner = udp[i + 4:i + 4 + l]
                i += 4 + l + (4 - l % 4) % 4
            how = "ind"
    if inner is None:
        continue
    s = stun(inner)
    if s:
        kind = {0x0001: "req", 0x0101: "ok", 0x0111: "err"}.get(s[0], hex(s[0]))
        seen[("out" if out else "in ", how, kind, " ".join(x for x in s[1] if x not in ("MI", "FP", "USERNAME")))] += 1
    else:
        seen[("out" if out else "in ", how, "non-stun", str(inner[0]))] += 1
for k, v in sorted(seen.items()):
    print(v, *k)
