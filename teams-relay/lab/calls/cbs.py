"""Callbacks of an echo-mode debug log (lines {"cb":..,"t":..,"body":..}): time, name, body cut to N chars (argv[2]).
Lines {"offer":..}, {"mediaAnswer":..} and {"answer":..} are printed whole when argv[3] == "sdp"."""
import json, sys

cut = int(sys.argv[2]) if len(sys.argv) > 2 else 600
sdp = len(sys.argv) > 3 and sys.argv[3] == "sdp"
for line in open(sys.argv[1], errors="replace"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        v = json.loads(line)
    except Exception:
        continue
    if "cb" in v:
        body = json.dumps(v["body"])
        print(v["t"], v["cb"], body[:cut])
        print()
    elif sdp and ("offer" in v or "mediaAnswer" in v or "answer" in v):
        k = "offer" if "offer" in v else "mediaAnswer" if "mediaAnswer" in v else "answer"
        print(f"== {k}")
        print(v[k])
