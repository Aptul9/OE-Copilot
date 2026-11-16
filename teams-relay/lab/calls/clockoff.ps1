# Offset of the oe-vm clock against this laptop (ms, positive = oe-vm ahead): oe-vm prints its clock 10 times over
# one ssh session, each line timed here on arrival; the smallest (remote - local) is the offset less one-way latency.
$p = New-Object System.Diagnostics.Process
$p.StartInfo.FileName = "ssh"
$p.StartInfo.Arguments = 'oe-vm "for i in 1 2 3 4 5 6 7 8 9 10; do date +%s%3N; sleep 0.2; done"'
$p.StartInfo.RedirectStandardOutput = $true
$p.StartInfo.UseShellExecute = $false
[void]$p.Start()
$diffs = @()
while (-not $p.StandardOutput.EndOfStream) {
  $line = $p.StandardOutput.ReadLine()
  $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  if ($line -match '^\d+$') { $diffs += ([long]$line - $now) }
}
$p.WaitForExit()
$sorted = $diffs | Sort-Object
"offset_ms(remote-local, upper bound)={0} spread={1}..{2}" -f $sorted[-1], $sorted[0], $sorted[-1]
