param([string]$Pattern = '(?i)chiam|call|annulla|cancel', [int]$Depth = 3)
# Buttons of the Teams windows whose name matches, with the names of their ancestors (to tell a dialog from the chat).
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$scope = [System.Windows.Automation.TreeScope]
$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
$btn = New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
$pids = @((Get-Process ms-teams).Id)
foreach ($w in $A::RootElement.FindAll($scope::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
  if (-not ($pids -contains $w.Current.ProcessId)) { continue }
  "window: $($w.Current.Name)"
  foreach ($b in $w.FindAll($scope::Descendants, $btn)) {
    if ($b.Current.Name -notmatch $Pattern) { continue }
    $chain = @(); $p = $walker.GetParent($b)
    for ($i = 0; $i -lt $Depth -and $p; $i++) { $chain += "$($p.Current.ControlType.ProgrammaticName -replace 'ControlType.','')[$($p.Current.Name)]"; $p = $walker.GetParent($p) }
    "  button '$($b.Current.Name)' id=$($b.Current.AutomationId) <- $($chain -join ' <- ')"
  }
}
