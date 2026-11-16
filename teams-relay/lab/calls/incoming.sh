set -u
# Callee: debug Rust worker in serve mode as slot $SLOT (its live worker switched off meanwhile); caller: whoever rings it
SLOT=${SLOT:-6}
D=/var/tmp/calltest
sudo mkdir -p $D/serve && sudo chmod 700 $D/serve
sudo rm -f $D/serve/*
sudo install -m 755 /tmp/teamsrelay-calling-dbg $D/serve/teamsrelay-calling
sudo cp /tmp/harness.js $D/serve/harness.js
sudo cp /tmp/calls-flag.js $D/calls.js
sudo docker network create calltest > /dev/null 2>&1 || true
calls() { sudo docker exec -i -w /app teams-browsers node - "$1" "$2" < $D/calls.js; }
calls $SLOT 0
sleep 7
sudo docker logs --since 20s teams-browsers 2>&1 | grep -E "^\[$SLOT\].*call" | tail -2
sudo docker exec teams-browsers cat /root/data/$SLOT/discovery.json | sudo tee $D/serve/discovery.json > /dev/null
sudo chmod 600 $D/serve/discovery.json
echo "discovery age $(( $(date +%s) - $(sudo docker exec teams-browsers stat -c %Y /root/data/$SLOT/discovery.json) ))s"
BR=br-$(sudo docker network inspect -f '{{.Id}}' calltest | cut -c1-12)
sudo rm -f $D/cap-in.pcap
sudo timeout ${LIFE:-75} tcpdump -i $BR -U -s 0 -w $D/cap-in.pcap udp > /dev/null 2>&1 &
date -u +%H:%M:%S
sudo docker run --rm --name calltest-serve --network calltest -v $D/serve:/serve -e HOST_STATE=/serve -e BIN=/serve/teamsrelay-calling \
  -e CALLING_DEBUG=1 -e CALLING_NAME=TeamsRelay -e TALK=${TALK:-20} -e LIFE=${LIFE:-75} --entrypoint node teamsrelay-browsers /serve/harness.js 2>&1 \
  | sudo tee $D/incoming-$(date +%H%M%S).log | grep -vE '"callback"' | cut -c1-400
sudo pkill -f "tcpdump -i $BR"; sleep 1
calls $SLOT 1
echo "--- dtls/rtp"
sudo python3 -I /tmp/pcap.py $D/cap-in.pcap | awk '{k=$2" "$3" "$4" "$NF; c[k]++; if(!(k in f)) f[k]=$0} END {for (k in c) print c[k]"x", f[k]}' | sort -k2 -n | head -20
echo "--- ice"
sudo python3 -I /tmp/stunscan.py $D/cap-in.pcap
echo "--- rtp ssrc/pt"
sudo python3 -I /tmp/pcap.py $D/cap-in.pcap ssrc
echo "--- declared"
L=$(sudo ls -t $D | grep ^incoming- | head -1)
sudo grep -o 'x-ssrc-range:[0-9-]*' $D/$L | sort | uniq -c
