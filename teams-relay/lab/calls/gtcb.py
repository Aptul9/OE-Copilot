"""One Trouter callback of a Teams web trace by name (argv[2]), its body as JSON without the SDP blob, and the SDP."""
import base64, gzip, json, sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
want = sys.argv[2]
for x in d["net"]:
    if x["ev"] != "wsin" or not x["d"].startswith("3:::"):
        continue
    v = json.loads(x["d"][4:])
    name = (v.get("url") or "").split("?")[0].rstrip("/").split("/")[-1]
    if name != want:
        continue
    body = v.get("body", "")
    h = {k.lower(): val for k, val in (v.get("headers") or {}).items()}
    if h.get("x-microsoft-skype-content-encoding", "").lower() == "gzip":
        body = gzip.decompress(base64.b64decode(body)).decode()
    b = json.loads(body)
    blob = None
    def walk(o):
        global blob
        if isinstance(o, dict):
            for k in list(o):
                if k == "blob":
                    blob = o.pop(k)
                else:
                    walk(o[k])
        elif isinstance(o, list):
            for s in o:
                walk(s)
    walk(b)
    print(x["t"] - d["t0"], json.dumps(b, indent=1)[:6000])
    if blob:
        print("== sdp")
        print(blob)
    print("== headers", json.dumps({k: v for k, v in h.items() if "token" not in k}, indent=1)[:1500])
