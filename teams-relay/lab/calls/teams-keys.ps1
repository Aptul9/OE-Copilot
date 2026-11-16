param([string]$Keys = '^%+1', [long]$Hwnd = 0)
# Sends a key chord to the main window of new Teams (default Ctrl+Alt+Shift+1, "collect support files").
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
}
'@
Add-Type -AssemblyName System.Windows.Forms
if ($Hwnd -eq 0) {
  Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
  $A = [System.Windows.Automation.AutomationElement]
  $pids = @((Get-Process ms-teams).Id)
  foreach ($w in $A::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
    if ($pids -contains $w.Current.ProcessId -and $w.Current.Name -match 'Microsoft Teams$') { $Hwnd = $w.Current.NativeWindowHandle }
  }
}
if ($Hwnd -eq 0) { "no Teams main window"; exit 1 }
[void][W]::ShowWindow([IntPtr]$Hwnd, 9)
# an Alt tap lets this process move the foreground
[W]::keybd_event(0x12, 0, 0, [IntPtr]::Zero); [W]::keybd_event(0x12, 0, 2, [IntPtr]::Zero)
[void][W]::SetForegroundWindow([IntPtr]$Hwnd)
Start-Sleep -Milliseconds 700
"foreground ok: {0}" -f ([W]::GetForegroundWindow() -eq [IntPtr]$Hwnd)
[System.Windows.Forms.SendKeys]::SendWait($Keys)
"sent $Keys"
