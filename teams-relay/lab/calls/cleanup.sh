sudo docker ps -a --format '{{.Names}}' | grep '^calltest' && echo "calltest container left" || echo "no calltest container"
sudo docker network rm calltest > /dev/null && echo "network calltest removed"
sudo rm -rf /var/tmp/calltest
# the helpers copied to /tmp for the tests (README.md)
for f in teamsrelay-calling-dbg calls-flag.js harness.js answered.py cb.py cbs.py dtlscert.py firstrtp.py hello.py pcap.py receipt.py stunscan.py stuntrace.py turn.py; do sudo rm -f /tmp/$f; done
ls -d /var/tmp/calltest 2>/dev/null; ls /tmp | grep -E 'calls-flag|calling-dbg|harness|answered|cbs?\.py|dtlscert|firstrtp|hello|pcap|receipt|stun|turn\.py' || echo "tmp clean"
cat > /tmp/slots.js <<'EOF'
const D = require("/app/node_modules/better-sqlite3");
const db = new D("/root/data/app.db", { readonly: true });
console.log(db.prepare("select slot, calls from teams_accounts order by slot").all().map((r) => `${r.slot}:${r.calls}`).join(" "));
EOF
sudo docker exec -i -w /app teams-browsers node - < /tmp/slots.js; rm -f /tmp/slots.js
sudo docker exec teams-browsers sh -c 'ps -eo etime,args | grep "teamsrelay-calling --mode=serve" | grep -v grep'
sudo docker logs --since 15m teams-browsers 2>&1 | grep -E '^\[(1|2|6|7|11)\].*call' | tail -8
