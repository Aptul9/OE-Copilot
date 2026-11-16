set -u
# Reference: the Node worker (Teams web calling stack + libwebrtc) calling PEER from slot CALLER, captured on calltest
CALLER=${CALLER:-1}
PEER=${PEER:-9ceee9f5-6eb1-4585-a155-582808df989a}
D=/var/tmp/calltest
sudo mkdir -p $D/node && sudo chmod 700 $D/node
sudo rm -f $D/node/*
sudo docker exec teams-browsers cat /root/data/$CALLER/discovery.json | sudo tee $D/node/discovery.json > /dev/null
sudo chmod 600 $D/node/discovery.json
age=$(( $(date +%s) - $(sudo docker exec teams-browsers stat -c %Y /root/data/$CALLER/discovery.json) ))
echo "caller discovery age ${age}s"
[ $age -lt 840 ] || { echo "discovery too old for the Node host (15 min)"; exit 1; }
OID=$(sudo python3 -I -c "import json,base64,sys;t=json.load(open('$D/node/discovery.json'))['tokens']['skypeToken'];print(json.loads(base64.urlsafe_b64decode(t.split('.')[1]+'=='))['skypeid'].split(':')[1])")
BR=br-$(sudo docker network inspect -f '{{.Id}}' calltest | cut -c1-12)
sudo rm -f $D/cap-node.pcap
sudo timeout 60 tcpdump -i $BR -U -s 0 -w $D/cap-node.pcap udp > /dev/null 2>&1 &
sleep 1
date -u +%H:%M:%S
sudo docker run --rm --name calltest-node-echo --network calltest -v $D/node:/state -e HOST_STATE=/state -e HOST_BUNDLES=/app/calling -e WRTC_DIR=/app -e CALLING_OID=$OID \
  --entrypoint sh teamsrelay-browsers -c "sed 's/^const ECHO_BOT = .*/const ECHO_BOT = \"8:orgid:$PEER\";/' /app/calling-live-host.mjs > /app/calling-peer.mjs && node /app/calling-peer.mjs --mode=echo --talk-seconds=30" 2>&1 | grep -vE 'Experimental|trace-warnings' | cut -c1-300 | tail -25
sleep 1
sudo pkill -f "tcpdump -i $BR"; sleep 1
echo "--- dtls"
sudo python3 -I /tmp/pcap.py $D/cap-node.pcap | grep DTLS | head -12
echo "--- ice"
sudo python3 -I /tmp/stunscan.py $D/cap-node.pcap
sudo python3 -I /tmp/receipt.py $D/node/host-receipt.json 2>/dev/null | head -8
