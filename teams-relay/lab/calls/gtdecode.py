"""The calling traffic of a Teams web trace (gt-web-net.json from pw-hook.js): HTTP requests with their bodies and the
Trouter requests (socket.io 3::: frames, gzip bodies decoded), in time order from t0. argv[2]: cut (default 3000)."""
import base64, gzip, json, sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
cut = int(sys.argv[2]) if len(sys.argv) > 2 else 3000
t0 = d["t0"]
for x in d["net"]:
    t = x["t"] - t0
    if t < -2000:
        continue
    if x["ev"] == "req" and x.get("m") not in ("OPTIONS",) and "onecdn" not in x["url"] and "static" not in x["url"]:
        print(f"{t:7} HTTP {x['m']} {x['url'][:200]}")
        if x.get("body"):
            print("   body:", x["body"][:cut])
    elif x["ev"] == "res" and "onecdn" not in x["url"] and "static" not in x["url"]:
        print(f"{t:7} HTTP {x['status']} <- {x['url'][:120]}")
    elif x["ev"] in ("wsin", "wsout"):
        p = x["d"]
        if p.startswith("3:::"):
            try:
                v = json.loads(p[4:])
            except Exception:
                print(f"{t:7} {x['ev']} unparsable {p[:200]}")
                continue
            body = v.get("body", "")
            h = {k.lower(): val for k, val in (v.get("headers") or {}).items()}
            if h.get("x-microsoft-skype-content-encoding", "").lower() == "gzip" and body:
                try:
                    body = gzip.decompress(base64.b64decode(body)).decode("utf-8", "replace")
                except Exception as e:
                    body = f"<gzip error {e}>"
            name = (v.get("url") or "").split("?")[0].rstrip("/").split("/")[-1]
            print(f"{t:7} {x['ev']} {v.get('method', '')} {name or ('status ' + str(v.get('status')))} {v.get('url', '')[:160]}")
            if body:
                print("   body:", body[:cut])
        elif p.startswith("5:"):
            print(f"{t:7} {x['ev']} event {p[:300]}")
