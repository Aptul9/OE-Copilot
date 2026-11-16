"""Callbacks of a serve-mode debug log: name, and for the interesting ones the fields that say why."""
import json, sys
for line in open(sys.argv[1], errors="replace"):
    line = line.strip()
    if not line.startswith('{"callback"'):
        continue
    try:
        v = json.loads(line)
    except Exception:
        print("unparsable", line[:200])
        continue
    b = v.get("body") or {}
    name = v["callback"]
    keys = list(b.keys())[:8]
    extra = ""
    if name in ("end", "conversationEnd", "callEnd") or "callEnd" in b or "conversationEnd" in b:
        extra = json.dumps({k: b.get(k) for k in ("callEnd", "conversationEnd", "debugContent") if k in b})[:900]
    elif name not in ("rosterUpdate",):
        extra = json.dumps(b)[:500]
    print(name, keys, extra)
