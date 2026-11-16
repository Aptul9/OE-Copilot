"""The call-create body (POST epconv) of a Teams web trace, pretty, with the offer SDP printed as lines."""
import json, sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
for x in d["net"]:
    if x["ev"] == "req" and x.get("m") == "POST" and x["url"].endswith("/epconv") and x.get("body"):
        b = json.loads(x["body"])
        offer = b["callInvitation"]["mediaContent"].pop("blob")
        def strip(v):
            if isinstance(v, dict):
                return {k: ("<link>" if isinstance(s, str) and s.startswith("https://") else strip(s)) for k, s in v.items()}
            if isinstance(v, list):
                return [strip(s) for s in v]
            return v
        print(json.dumps(strip(b), indent=1))
        print("== offer")
        print(offer)
        print("== headers")
        print(json.dumps({k: v for k, v in x["h"].items() if k.lower() not in ("x-skypetoken", "authorization", "cookie")}, indent=1))
        break
