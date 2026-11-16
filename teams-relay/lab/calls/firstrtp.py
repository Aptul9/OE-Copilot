"""Epoch (ms) of the last DTLS handshake record and of the first inbound RTP audio packet (payload type 0-127 except
RTCP 72-76) of a capture, through TURN or direct. argv[2] (optional): the Accept press in unix ms on the oe-vm clock,
to print the delay from it."""
import struct, sys

def payload(udp):
    if len(udp) >= 4 and 0x40 <= udp[0] <= 0x7F:
        return udp[4:4 + struct.unpack("!H", udp[2:4])[0]]
    if len(udp) >= 20 and udp[0] < 2 and struct.unpack("!H", udp[:2])[0] in (0x16, 0x17):
        i = 20
        while i + 4 <= len(udp):
            t, l = struct.unpack("!HH", udp[i:i + 4])
            if t == 0x13:
                return udp[i + 4:i + 4 + l]
            i += 4 + l + (4 - l % 4) % 4
        return b""
    return udp

f = open(sys.argv[1], "rb")
f.read(24)
dtls_done = first_rtp = None
while True:
    h = f.read(16)
    if len(h) < 16:
        break
    ts, us, cap, _ = struct.unpack("<IIII", h)
    eth = f.read(cap)[14:]
    if len(eth) < 20 or eth[9] != 17:
        continue
    src = ".".join(map(str, eth[12:16]))
    if src.startswith("172."):
        continue
    p = payload(eth[(eth[0] & 15) * 4 + 8:])
    t = ts * 1000 + us // 1000
    if p and p[0] in (20, 22) and len(p) > 13:
        dtls_done = t
    if p and 128 <= p[0] <= 191 and len(p) > 12 and not 72 <= (p[1] & 0x7F) <= 76 and first_rtp is None and dtls_done:
        first_rtp = t
print("dtls_last_ms", dtls_done, "first_rtp_ms", first_rtp)
if len(sys.argv) > 2 and first_rtp:
    print("first audio", first_rtp - int(sys.argv[2]), "ms after Accept")
