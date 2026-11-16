import json, sys
r = json.load(open(sys.argv[1]))
for pc in r.get("peerConnections", []):
    print("pc", pc.get("iceServers"), pc.get("policy"), pc.get("states"))
    for s in pc.get("stats", [])[-3:]:
        print(" stat", json.dumps(s)[:1500])
print("audio", json.dumps(r.get("audio"))[:300])
