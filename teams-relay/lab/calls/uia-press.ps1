param([string]$Dialog = 'Avviare la chiamata?', [string]$Button = 'Avvia chiamata')
# Presses one button inside one named dialog of new Teams; nothing else.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$scope = [System.Windows.Automation.TreeScope]
$isDialog = New-Object System.Windows.Automation.AndCondition(
  (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Window)),
  (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $Dialog)))
$isButton = New-Object System.Windows.Automation.AndCondition(
  (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)),
  (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $Button)))
$pids = @((Get-Process ms-teams).Id)
foreach ($w in $A::RootElement.FindAll($scope::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
  if (-not ($pids -contains $w.Current.ProcessId)) { continue }
  $d = $w.FindFirst($scope::Descendants, $isDialog)
  if (-not $d) { continue }
  $b = $d.FindFirst($scope::Descendants, $isButton)
  if ($b) { $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); "{0:HH:mm:ss.fff} pressed '$Button' in '$Dialog'" -f (Get-Date); exit 0 }
}
"dialog or button not found"
exit 1
