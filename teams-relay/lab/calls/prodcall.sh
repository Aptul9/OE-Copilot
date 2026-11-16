set -u
# A real outgoing call through the app path on prod: a 'call' command queued in slot $SLOT's messages.db (as the web
# app queues it), hung up after $TALK s with a 'hangup' command; UDP of teams-browsers captured on its bridge.
SLOT=${SLOT:-1}
CHAT=${CHAT:-Antonio Maritato | Ext Vection Technologies}
TALK=${TALK:-25}
D=/var/tmp/calltest
sudo mkdir -p $D && sudo chmod 700 $D
NET=$(sudo docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' teams-browsers | awk '{print $1}')
BR=br-$(sudo docker network inspect -f '{{.Id}}' $NET | cut -c1-12)
echo "net $NET bridge $BR"
cat > /tmp/cmd.js <<'EOF'
const D = require("/app/node_modules/better-sqlite3");
const [slot, type, arg1, arg2] = process.argv.slice(2);
const db = new D(`/root/data/${slot}/messages.db`);
const r = db.prepare("INSERT INTO commands(ts, type, arg1, arg2) VALUES(?,?,?,?)").run(Math.floor(Date.now() / 1000), type, arg1, arg2);
console.log(type, "queued", r.lastInsertRowid, Date.now());
EOF
TAG=$(date +%H%M%S)
sudo timeout $((TALK + 30)) tcpdump -i $BR -U -s 0 -w $D/prod-$TAG.pcap udp > /dev/null 2>&1 &
sleep 1
sudo docker exec -i -w /app teams-browsers node - "$SLOT" call "$CHAT" '{"audio":true}' < /tmp/cmd.js
sleep $((TALK + 8))
sudo docker exec -i -w /app teams-browsers node - "$SLOT" hangup "$CHAT" '' < /tmp/cmd.js
sleep 6
sudo pkill -f "tcpdump -i $BR"; sleep 1
rm -f /tmp/cmd.js
sudo docker logs --since $((TALK + 60))s teams-browsers 2>&1 | grep -E "^\[$SLOT\].*(call|media)" | tail -6
echo "pcap $D/prod-$TAG.pcap"
