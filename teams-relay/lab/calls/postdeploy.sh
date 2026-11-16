sleep 20
date -u +%H:%M:%S
sudo cat /opt/teamsrelay/.deployed-sha 2>/dev/null || sudo find /opt -maxdepth 2 -name .deployed-sha -exec sh -c 'echo "$1: $(cat $1)"' _ {} \;
sudo docker exec teams-browsers /app/teamsrelay-calling --check
sudo docker exec teams-browsers sh -c 'ps -eo etime,args' | grep 'teamsrelay-calling' | grep -v grep
sudo docker logs --since 3m teams-browsers 2>&1 | grep -iE 'error|panic|fatal|warn' | tail -8
cat > /tmp/health.js <<'EOF'
const D = require("/app/node_modules/better-sqlite3");
const out = [];
for (const n of require("fs").readdirSync("/root/data").filter((d) => /^\d+$/.test(d)).sort((a, b) => a - b)) {
  try {
    const db = new D(`/root/data/${n}/messages.db`, { readonly: true });
    const h = db.prepare("select v from state where k='health'").get();
    const v = h ? JSON.parse(h.v) : {};
    out.push(`${n}:${v.teams ?? "?"}/${v.overall ?? "?"}`);
  } catch (e) { out.push(`${n}:err`); }
}
console.log(out.join(" "));
EOF
sudo docker exec -i -w /app teams-browsers node - < /tmp/health.js; rm -f /tmp/health.js
echo "--- echo call with the deployed binary (slot 2)"
sudo docker exec -e CALLING_DEBUG=1 teams-browsers timeout 60 /app/teamsrelay-calling --mode=echo --discovery=/root/data/${ECHO_SLOT:-2}/discovery.json --seconds=30 2>&1 | grep -E '"step":"(relay|accepted|done)"|"candidate":"Relay'
