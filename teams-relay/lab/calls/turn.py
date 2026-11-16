"""TURN Allocate against the Teams relay with the credentials of its token service (as Teams web gets them)."""
import base64, hashlib, hmac, json, os, socket, struct, sys, urllib.request, zlib

disc = json.load(open(sys.argv[1]))
tok = disc["tokens"]["skypeToken"]
claims = json.loads(base64.urlsafe_b64decode(tok.split(".")[1] + "=="))
oid = claims["skypeid"].split(":")[1]
tid = claims["tid"]
ecs = f"https://config.teams.microsoft.com/config/v1/Skype/1415_1.0.0.0?tenantId={tid}&aadUserId={oid}&userId={oid}&region={disc['region']}&agents=MDN_TRAP&ECSCanary=1"
cfg = json.load(urllib.request.urlopen(urllib.request.Request(ecs, headers={"X-ECS-Client-Last-Telemetry-Events": "ecs_client_library_name=@1js/ecs-client,ecs_client_app_name=SkypeEcsConfigAdapter,ecs_client_version=1.2.0"})))
trap = cfg["MDN_TRAP"]
turn = trap["Relay"]["Turn"]
print("turn config", json.dumps({k: turn[k] for k in turn if k != "credentials"})[:400])
req = urllib.request.Request(trap["Service"]["tokenUrl"], headers={"api-version": "2", "Content-Type": "application/json", "X-Skypetoken": tok, "X-MS-Migration": "True"})
t = json.load(urllib.request.urlopen(req))["tokens"][0]
user, pw = t["username"], t["password"]
print("token keys", sorted(t.keys()), "username bytes", len(user.encode()), "password bytes", len(pw.encode()), "realm" in t and t.get("realm"))

MAGIC = 0x2112A442
def attr(tp, val):
    pad = (4 - len(val) % 4) % 4
    return struct.pack("!HH", tp, len(val)) + val + b"\0" * pad
def msg(mtype, attrs, key=None):
    tid = os.urandom(12)
    body = b"".join(attrs)
    if key is not None:
        hdr = struct.pack("!HHI", mtype, len(body) + 24, MAGIC) + tid
        mac = hmac.new(key, hdr + body, hashlib.sha1).digest()
        body += attr(0x0008, mac)
    hdr = struct.pack("!HHI", mtype, len(body), MAGIC) + tid
    return hdr + body
def parse(data):
    mtype, ln, _ = struct.unpack("!HHI", data[:8])
    out, i = {}, 20
    while i + 4 <= 20 + ln:
        tp, l = struct.unpack("!HH", data[i:i + 4])
        out[tp] = data[i + 4:i + 4 + l]
        i += 4 + l + (4 - l % 4) % 4
    return mtype, out
def xaddr(v):
    port = struct.unpack("!H", v[2:4])[0] ^ (MAGIC >> 16)
    ip = ".".join(str(b ^ m) for b, m in zip(v[4:8], struct.pack("!I", MAGIC)))
    return f"{ip}:{port}"

host = turn["fqdns"][0]
port = turn.get("udpPort", 3478)
addr = (socket.gethostbyname(host), port)
print("relay", host, addr)
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(3)
transport = attr(0x0019, bytes([17, 0, 0, 0]))
s.sendto(msg(0x0003, [transport]), addr)
mtype, a = parse(s.recvfrom(2048)[0])
err = a.get(0x0009, b"")
print("allocate 1:", hex(mtype), "error", err[2] * 100 + err[3] if err else None, err[4:].decode(errors="replace"), "realm", a.get(0x0014), "nonce" in str(a.keys()) or bool(a.get(0x0015)))
realm, nonce = a.get(0x0014, b""), a.get(0x0015, b"")
key = hashlib.md5(user.encode() + b":" + realm + b":" + pw.encode()).digest()
s.sendto(msg(0x0003, [transport, attr(0x0006, user.encode()), attr(0x0014, realm), attr(0x0015, nonce)], key), addr)
mtype, a = parse(s.recvfrom(2048)[0])
err = a.get(0x0009, b"")
print("allocate 2:", hex(mtype), "error", err[2] * 100 + err[3] if err else None, err[4:].decode(errors="replace"), "relayed", xaddr(a[0x0016]) if 0x0016 in a else None, "mapped", xaddr(a[0x0020]) if 0x0020 in a else None)

def plain_addr(v):
    return ".".join(str(b) for b in v[4:8]), struct.unpack("!H", v[2:4])[0]
for n in range(3):
    alt = a.get(0x8023)
    if not alt:
        break
    addr = plain_addr(alt)
    print("alternate server", addr, "attrs", sorted(hex(k) for k in a))
    s.sendto(msg(0x0003, [transport]), addr)
    mtype, a = parse(s.recvfrom(2048)[0])
    err = a.get(0x0009, b"")
    print(f"alt {n} allocate 1:", hex(mtype), "error", err[2] * 100 + err[3] if err else None, err[4:].decode(errors="replace"), "realm", a.get(0x0014))
    realm, nonce = a.get(0x0014, realm), a.get(0x0015, b"")
    key = hashlib.md5(user.encode() + b":" + realm + b":" + pw.encode()).digest()
    s.sendto(msg(0x0003, [transport, attr(0x0006, user.encode()), attr(0x0014, realm), attr(0x0015, nonce)], key), addr)
    mtype, a = parse(s.recvfrom(2048)[0])
    err = a.get(0x0009, b"")
    print(f"alt {n} allocate 2:", hex(mtype), "error", err[2] * 100 + err[3] if err else None, err[4:].decode(errors="replace"), "relayed", xaddr(a[0x0016]) if 0x0016 in a else None, "mapped", xaddr(a[0x0020]) if 0x0020 in a else None)
    if mtype == 0x0103:
        # free the allocation: Refresh with lifetime 0
        s.sendto(msg(0x0004, [attr(0x000D, struct.pack("!I", 0)), attr(0x0006, user.encode()), attr(0x0014, realm), attr(0x0015, nonce)], key), addr)
        print("refresh 0:", hex(parse(s.recvfrom(2048)[0])[0]))
        break
