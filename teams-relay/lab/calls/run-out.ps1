param([int]$Runs = 1, [string]$Peer = '9ceee9f5-6eb1-4585-a155-582808df989a', [int]$Talk = 30, [int]$DelayMs = 3000, [string]$Env = '')
# Outgoing calls slot 1 -> $Peer on the laptop's Teams desktop, one after another: uia-accept.ps1 presses Accept after
# $DelayMs of ringing, call.sh places the call on oe-vm, firstrtp.py gives the first inbound RTP after DTLS against
# the press (laptop and oe-vm clocks agree within ~50 ms, clockoff.ps1). One summary line per run.
$lab = Split-Path -Parent $MyInvocation.MyCommand.Path
$script = (Get-Content -Raw "$lab\call.sh") -replace "`r", ""
for ($i = 1; $i -le $Runs; $i++) {
  $uiaOut = Join-Path $env:TEMP "uia-run$i.txt"
  $uia = Start-Process powershell -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$lab\uia-accept.ps1`" -Seconds $($Talk + 30) -DelayMs $DelayMs" -RedirectStandardOutput $uiaOut -WindowStyle Hidden -PassThru
  Start-Sleep -Seconds 3
  $out = $script | ssh oe-vm "SECONDS_TALK=$Talk PEER=$Peer $Env bash -s"
  $uia.WaitForExit(90000) | Out-Null
  $press = ([regex]::Match((Get-Content -Raw $uiaOut), 'unix_ms=(\d+)')).Groups[1].Value
  $done = ($out | Select-String '"step":"done"').Line
  $acc = ($out | Select-String '"step":"accepted"').Line
  $pcap = ([regex]::Match(($out -join "`n"), 'pcap (\S+)')).Groups[1].Value
  $first = if ($press -and $pcap) { ssh oe-vm "sudo python3 -I /tmp/firstrtp.py $pcap $press" } else { "no press or pcap" }
  "run $i press=$press"
  "  accepted: $acc"
  "  done: $done"
  "  $($first -join ' | ')"
  if (-not $done) { $out | Select-Object -Last 8 }
  Start-Sleep -Seconds 5
}
