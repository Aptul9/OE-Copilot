param([int]$Seconds = 60, [switch]$DryRun, [int]$DelayMs = 0)
# Waits for a new top-level window of new Teams (the incoming-call notification) and presses its accept-with-audio
# button. Windows open before the start (the main window) are never touched.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$root = $A::RootElement
$scope = [System.Windows.Automation.TreeScope]
$btnCond = New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
function TeamsWindows {
  $pids = @((Get-Process ms-teams -ErrorAction SilentlyContinue).Id)
  $root.FindAll($scope::Children, [System.Windows.Automation.Condition]::TrueCondition) | Where-Object { $pids -contains $_.Current.ProcessId }
}
$initial = @(TeamsWindows | ForEach-Object { $_.Current.NativeWindowHandle })
"{0:HH:mm:ss.fff} watching, {1} Teams windows already open" -f (Get-Date), $initial.Count
$end = (Get-Date).AddSeconds($Seconds)
$logged = @{}
while ((Get-Date) -lt $end) {
  foreach ($w in TeamsWindows) {
    $h = $w.Current.NativeWindowHandle
    if ($initial -contains $h) { continue }
    $buttons = @($w.FindAll($scope::Descendants, $btnCond))
    $names = ($buttons | ForEach-Object { $_.Current.Name }) -join ' / '
    if (-not $logged.ContainsKey("$h$names")) { $logged["$h$names"] = 1; "{0:HH:mm:ss.fff} new window '{1}': {2}" -f (Get-Date), $w.Current.Name, $names }
    foreach ($b in $buttons) {
      if ($b.Current.Name -match '(?i)^(accetta|accept)$|(accetta|accept|rispondi|answer).*audio') {
        if ($DelayMs -gt 0) { "{0:HH:mm:ss.fff} ringing, waiting {1} ms" -f (Get-Date), $DelayMs; Start-Sleep -Milliseconds $DelayMs }
        "{0:HH:mm:ss.fff} pressing '{1}' unix_ms={2}" -f (Get-Date), $b.Current.Name, [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        if (-not $DryRun) { $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }
        return
      }
    }
  }
  Start-Sleep -Milliseconds 250
}
"no accept button within $Seconds s"
