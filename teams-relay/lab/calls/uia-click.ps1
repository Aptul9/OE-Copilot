param([int]$Seconds = 60, [int]$DelayMs = 3000, [string]$Button = '(?i)^(accetta|accept)$|(accetta|accept).*audio')
# As uia-accept.ps1, but the accept button gets a real mouse click (cursor moved onto it, left down/up) instead of
# the UIA Invoke pattern. Windows open before the start are never touched.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class M {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
'@
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
while ((Get-Date) -lt $end) {
  foreach ($w in TeamsWindows) {
    $h = $w.Current.NativeWindowHandle
    if ($initial -contains $h) { continue }
    $buttons = @($w.FindAll($scope::Descendants, $btnCond))
    foreach ($b in $buttons) {
      if ($b.Current.Name -match $Button) {
        "{0:HH:mm:ss.fff} new window '{1}': {2}" -f (Get-Date), $w.Current.Name, (($buttons | ForEach-Object { $_.Current.Name }) -join ' / ')
        Start-Sleep -Milliseconds $DelayMs
        $r = $b.Current.BoundingRectangle
        $x = [int]($r.X + $r.Width / 2); $y = [int]($r.Y + $r.Height / 2)
        [void][M]::SetForegroundWindow([IntPtr]$h)
        [void][M]::SetCursorPos($x, $y)
        Start-Sleep -Milliseconds 150
        [M]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 60
        [M]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
        "{0:HH:mm:ss.fff} clicked '{1}' at {2},{3}" -f (Get-Date), $b.Current.Name, $x, $y
        Start-Sleep -Milliseconds 1500
        foreach ($w2 in TeamsWindows) { if ($initial -notcontains $w2.Current.NativeWindowHandle) { "{0:HH:mm:ss.fff} window after click: '{1}'" -f (Get-Date), $w2.Current.Name } }
        return
      }
    }
  }
  Start-Sleep -Milliseconds 250
}
"no accept button within $Seconds s"
