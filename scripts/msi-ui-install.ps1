<#
.SYNOPSIS
Installs the Windows MSI through its own dialogs, driven by UI Automation.

.DESCRIPTION
Starts `msiexec /i <msi> /l*v <log>` with the full UI, chooses Just for me (-Scope user)
or For all users (-Scope machine) in ScopeDlg with a real mouse click, presses the Install
button that the choice shows, waits for ExitDlg and presses Finish. Exits with msiexec's
exit code, or 1 when a dialog does not appear in time (msiexec is then stopped and a
screenshot saved). It installs for real: scripts/test-msi-install.py runs it only on a
disposable machine (--disposable-machine), and it needs an interactive desktop and an
elevated administrator for -Scope machine (no UAC prompt can be answered).
Windows PowerShell 5.1 (UI Automation of .NET Framework).
#>
param(
  [Parameter(Mandatory)] [string]$Msi,
  [Parameter(Mandatory)] [ValidateSet('user', 'machine')] [string]$Scope,
  [Parameter(Mandatory)] [string]$Log,
  [Parameter(Mandatory)] [string]$Screenshot,
  [int]$TimeoutSeconds = 300
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type -Namespace MidnightMsiTest -Name Mouse -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, System.UIntPtr extra);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr window);
'@

$automation = [System.Windows.Automation.AutomationElement]
$tree = [System.Windows.Automation.TreeScope]

function Save-Screenshot {
  try {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bitmap.Save($Screenshot, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
  } catch { Write-Warning "No screenshot: $_" }
}

function Get-Windows([int]$ProcessId) {
  $condition = New-Object System.Windows.Automation.PropertyCondition($automation::ProcessIdProperty, $ProcessId)
  @($automation::RootElement.FindAll($tree::Children, $condition))
}

# Controls of the process's windows whose name (without the & of access keys) matches.
function Find-Control([int]$ProcessId, [string]$Pattern, [System.Windows.Automation.ControlType]$Type) {
  foreach ($window in Get-Windows $ProcessId) {
    $condition = New-Object System.Windows.Automation.PropertyCondition($automation::ControlTypeProperty, $Type)
    foreach ($element in $window.FindAll($tree::Descendants, $condition)) {
      $name = $element.Current.Name -replace '&', ''
      if ($name -match $Pattern -and -not $element.Current.IsOffscreen -and $element.Current.IsEnabled) {
        return [pscustomobject]@{ Window = $window; Element = $element }
      }
    }
  }
  $null
}

function Wait-Control([System.Diagnostics.Process]$Process, [string]$Pattern, [System.Windows.Automation.ControlType]$Type) {
  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    if ($Process.HasExited) { throw "msiexec ended with $($Process.ExitCode) before '$Pattern' appeared" }
    $found = Find-Control $Process.Id $Pattern $Type
    if ($found) { return $found }
    Start-Sleep -Milliseconds 500
  }
  throw "'$Pattern' did not appear within $TimeoutSeconds seconds"
}

function Invoke-Button($Found) {
  $pattern = $Found.Element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
  $pattern.Invoke()
}

# A real click, so that Windows Installer sees the radio button's notification and
# applies the dialog's control conditions (which Install button is shown).
function Click-Element($Found) {
  [void][MidnightMsiTest.Mouse]::SetForegroundWindow([System.IntPtr]$Found.Window.Current.NativeWindowHandle)
  $Found.Element.SetFocus()
  $point = $Found.Element.GetClickablePoint()
  [void][MidnightMsiTest.Mouse]::SetCursorPos([int]$point.X, [int]$point.Y)
  [MidnightMsiTest.Mouse]::mouse_event(0x2, 0, 0, 0, [System.UIntPtr]::Zero)
  [MidnightMsiTest.Mouse]::mouse_event(0x4, 0, 0, 0, [System.UIntPtr]::Zero)
}

$button = [System.Windows.Automation.ControlType]::Button
$radio = [System.Windows.Automation.ControlType]::RadioButton
$process = Start-Process -FilePath (Join-Path ([Environment]::SystemDirectory) 'msiexec.exe') `
  -ArgumentList @('/i', "`"$Msi`"", '/l*v', "`"$Log`"") -PassThru
try {
  $choice = if ($Scope -eq 'user') { '^Just for me' } else { '^For all users' }
  $option = Wait-Control $process $choice $radio
  Write-Output "ScopeDlg: choosing '$($option.Element.Current.Name)'"
  Click-Element $option
  Start-Sleep -Milliseconds 500
  $selection = $option.Element.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
  if (-not $selection.Current.IsSelected) { throw "The $Scope choice is not selected after the click" }
  $install = Wait-Control $process '^Install$' $button
  Invoke-Button $install
  Write-Output 'ScopeDlg: Install pressed'
  $finish = Wait-Control $process '^Finish$' $button
  $texts = @($finish.Window.FindAll($tree::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
    ForEach-Object { $_.Current.Name } | Where-Object { $_ })
  Write-Output ("Closing dialog: " + ($texts -join ' | '))
  Save-Screenshot
  Invoke-Button $finish
  if (-not $process.WaitForExit($TimeoutSeconds * 1000)) { throw 'msiexec did not end after Finish' }
  if (-not ($texts -match 'has finished')) { throw 'The closing dialog is not the success dialog' }
  Write-Output "msiexec ended with $($process.ExitCode)"
  exit $process.ExitCode
} catch {
  Write-Output "UI installation failed: $_"
  Save-Screenshot
  if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
  exit 1
}
