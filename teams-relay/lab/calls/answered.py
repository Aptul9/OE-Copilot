"""Inbound ICE checks of a capture (direct, ChannelData, Data indication) by USERNAME and source, and how many this end
answered (a response with the same transaction id going out)."""
import struct, sys
from collections import defaultdict

def stun(b):
    if len(b) < 20 or b[0] >= 2 or b[4:8] != b"\x21\x12\xa4\x42":
        return None
    t = struct.unpack("!H", b[:2])[0]
    attrs, i = {}, 20
    while i + 4 <= len(b):
        a, l = struct.unpack("!HH", b[i:i + 4])
        attrs[a] = b[i + 4:i + 4 + l]
        i += 4 + l + (4 - l % 4) % 4
    return t, b[8:20], attrs

def unwrap(udp):
    if 0x40 <= udp[0] <= 0x7F:
        return "chan", udp[4:4 + struct.unpack("!H", udp[2:4])[0]], None
    s = stun(udp)
    if s and s[0] in (0x0016, 0x0017):
        peer = s[2].get(0x0012)
        addr = None
        if peer:
            port = struct.unpack("!H", peer[2:4])[0] ^ 0x2112
            addr = ".".join(str(b ^ m) for b, m in zip(peer[4:8], b"\x21\x12\xa4\x42")) + f":{port}"
        return "ind", s[2].get(0x0013, b""), addr
    return "direct", udp, None

reqs = {}
answered = set()
f = open(sys.argv[1], "rb"); f.read(24)
while True:
    h = f.read(16)
    if len(h) < 16:
        break
    cap = struct.unpack("<IIII", h)[2]
    eth = f.read(cap)[14:]
    if len(eth) < 20 or eth[9] != 17:
        continue
    ihl = (eth[0] & 15) * 4
    src = ".".join(map(str, eth[12:16])); dst = ".".join(map(str, eth[16:20]))
    sp, dp = struct.unpack("!HH", eth[ihl:ihl + 4])
    out = src.startswith("172.")
    how, inner, peer = unwrap(eth[ihl + 8:])
    s = stun(inner)
    if not s:
        continue
    t, tid, attrs = s
    if not out and t == 0x0001:
        user = attrs.get(0x0006, b"").decode(errors="replace")
        reqs[tid] = (how, peer or f"{src}:{sp}", f"to :{dp}", user)
    if out and t == 0x0101:
        answered.add(tid)
summary = defaultdict(lambda: [0, 0])
for tid, k in reqs.items():
    summary[k][0] += 1
    summary[k][1] += tid in answered
for k, (n, a) in sorted(summary.items(), key=lambda x: -x[1][0]):
    print(f"{n:3} requests, {a:3} answered  {k[0]:6} from {k[1]:24} {k[2]:8} user {k[3]}")
