set -u
# One outgoing call from slot $CALLER to $PEER with the debug worker (echo mode) in a throwaway container, captured on
# the bridge of net calltest. Every CALLING_* variable of the environment goes into the container.
CALLER=${CALLER:-1}
PEER=${PEER:-9ceee9f5-6eb1-4585-a155-582808df989a}
SECONDS_TALK=${SECONDS_TALK:-50}
D=/var/tmp/calltest
sudo mkdir -p $D/dbg && sudo chmod 700 $D
sudo install -m 755 /tmp/teamsrelay-calling-dbg $D/dbg/teamsrelay-calling
sudo docker exec teams-browsers cat /root/data/$CALLER/discovery.json | sudo tee $D/dbg/discovery.json > /dev/null
sudo chmod 600 $D/dbg/discovery.json
echo "caller discovery age $(( $(date +%s) - $(sudo docker exec teams-browsers stat -c %Y /root/data/$CALLER/discovery.json) ))s"
sudo docker network create calltest > /dev/null 2>&1 || true
BR=br-$(sudo docker network inspect -f '{{.Id}}' calltest | cut -c1-12)
SUBNET=$(sudo docker network inspect -f '{{range .IPAM.Config}}{{.Subnet}}{{end}}' calltest)
echo "bridge $BR subnet $SUBNET"
ENVS=(-e CALLING_DEBUG=1)
for v in $(compgen -e | grep '^CALLING_'); do ENVS+=(-e "$v=${!v}"); done
echo "env ${ENVS[*]}"
TAG=$(date +%H%M%S)
sudo timeout $((SECONDS_TALK + 20)) tcpdump -i $BR -U -s 0 -w $D/cap-$TAG.pcap udp > /dev/null 2>&1 &
sleep 1
date -u +%H:%M:%S.%N | cut -c1-12
sudo docker run --rm --name calltest-rust --network calltest "${ENVS[@]}" -v $D/dbg:/dbg --entrypoint /dbg/teamsrelay-calling \
  teamsrelay-browsers --mode=echo --discovery=/dbg/discovery.json --peer=8:orgid:$PEER --seconds=$SECONDS_TALK > $D/native-$TAG.log 2>&1
sleep 1
sudo pkill -f "tcpdump -i $BR"; sleep 1
grep -E '"step":"(relay|offer|created|media-answer|early-applied|accepted|done)"|iceState|pcState|"ice":' $D/native-$TAG.log | cut -c1-600
echo "--- log $D/native-$TAG.log pcap $D/cap-$TAG.pcap"
