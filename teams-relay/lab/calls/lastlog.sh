# The latest echo-mode log on oe-vm without callback bodies, cut to 300 chars per line
L=$(sudo ls -t /var/tmp/calltest | grep '^native-' | head -1)
echo "$L"
sudo grep -v '^{"cb"' "/var/tmp/calltest/$L" | grep -v '"mediaAnswer"' | grep -v '"offer"' | cut -c1-300 | head -${N:-30}
