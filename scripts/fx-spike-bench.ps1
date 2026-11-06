param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Label,
  [int]$Repeats = 5,
  [int]$TreeWaitMs = 20000,
  [int]$IdleSeconds = 25,
  [switch]$DebugTree
)

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class NativeWin {
  [DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);

  delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);

  // winit creates a hidden "Winit Thread Event Target" window before the real one, and a
  // console-subsystem binary also owns a PseudoConsoleWindow; Process.MainWindowHandle can resolve
  // to either, so the app window must be found by class and title instead.
  public static IntPtr AppWindow(uint targetPid) {
    IntPtr pick = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid != targetPid || !IsWindowVisible(h)) { return true; }
      var c = new StringBuilder(256); GetClassName(h, c, 256);
      var t = new StringBuilder(256); GetWindowText(h, t, 256);
      string cls = c.ToString();
      if (cls == "Winit Thread Event Target" || cls == "PseudoConsoleWindow" || cls == "IME" || cls == "MSCTFIME UI") { return true; }
      if (t.Length == 0) { return true; }
      pick = h;
      return false;
    }, IntPtr.Zero);
    return pick;
  }
}
'@
$ErrorActionPreference = 'Stop'

# PowerShell variable names are case-insensitive: constants use the CT_ prefix so
# These constants must not share a name with any result variable.
$CT_LIST = [System.Windows.Automation.ControlType]::List
$CT_EDIT = [System.Windows.Automation.ControlType]::Edit
$CT_ITEM = [System.Windows.Automation.ControlType]::ListItem
$CT_TEXT = [System.Windows.Automation.ControlType]::Text

function Wait-AppWindow([Diagnostics.Process]$p, [int]$timeoutMs) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
    $h = [NativeWin]::AppWindow([uint32]$p.Id)
    if ($h -ne [IntPtr]::Zero) { return [pscustomobject]@{ Ms = $sw.ElapsedMilliseconds; Hwnd = $h } }
    Start-Sleep -Milliseconds 5
  }
  return [pscustomobject]@{ Ms = -1; Hwnd = [IntPtr]::Zero }
}

function Find-Type($scope, $ct) {
  if ($null -eq $scope) { return $null }
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $ct)
  return $scope.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
}

function Items($list) {
  if ($null -eq $list) { return $null }
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $CT_ITEM)
  return $list.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
}

function RowText($item) {
  if ($item.Current.Name) { return $item.Current.Name }
  $t = Find-Type $item $CT_TEXT
  if ($t -and $t.Current.Name) { return $t.Current.Name }
  return ''
}

# Accumulates wall-clock milliseconds during which the UI thread stops pumping messages.
function Measure-Hung([IntPtr]$hwnd, [int]$observeMs) {
  $total = 0.0
  $last = [DateTime]::UtcNow
  $outer = [Diagnostics.Stopwatch]::StartNew()
  while ($outer.ElapsedMilliseconds -lt $observeMs) {
    Start-Sleep -Milliseconds 5
    $now = [DateTime]::UtcNow
    $dt = ($now - $last).TotalMilliseconds
    $last = $now
    if ([NativeWin]::IsHungAppWindow($hwnd)) { $total += $dt }
  }
  return [Math]::Round($total, 1)
}

Write-Output "### BENCH $Label exe=$Exe repeats=$Repeats idle_seconds=$IdleSeconds"

for ($r = 0; $r -le $Repeats; $r++) {
  $tag = if ($r -eq 0) { 'warmup' } else { "run$r" }
  $p = Start-Process -FilePath $Exe -PassThru
  $w = Wait-AppWindow $p 60000
  $startup = $w.Ms
  $hwnd = $w.Hwnd
  if ($startup -lt 0) {
    Write-Output ("{0} NO_APP_WINDOW" -f $tag)
    Stop-Process -Id $p.Id -ErrorAction SilentlyContinue
    continue
  }

  $treeMs = -1; $list = $null; $edit = $null; $root = $null
  $swT = [Diagnostics.Stopwatch]::StartNew()
  $poll = 0
  while ($swT.ElapsedMilliseconds -lt $TreeWaitMs) {
    $h2 = [NativeWin]::AppWindow([uint32]$p.Id)
    if ($h2 -ne [IntPtr]::Zero) { $hwnd = $h2 }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    if ($root) {
      $list = Find-Type $root $CT_LIST
      $edit = Find-Type $root $CT_EDIT
      if ($DebugTree -and ($poll % 5) -eq 0) {
        $d = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        $inlineCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::List)
        $inline = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $inlineCond)
        Write-Output ("   poll{0} hwnd={1} class={2} name=[{3}] descendants={4} fn_list={5} inline_list={6} edit={7}" -f `
          $poll, $hwnd, $root.Current.ClassName, $root.Current.Name, $d.Count, ($null -ne $list), ($null -ne $inline), ($null -ne $edit))
      }
      if ($list -and $edit) { $treeMs = $swT.ElapsedMilliseconds; break }
    }
    $poll++
    Start-Sleep -Milliseconds 100
  }
  $rootCT = 'none'; $rootName = ''
  if ($root) { $rootCT = $root.Current.ControlType.ProgrammaticName; $rootName = $root.Current.Name }

  Start-Sleep -Seconds $IdleSeconds
  $p.Refresh()
  $memMb = [Math]::Round($p.WorkingSet64 / 1MB, 1)
  $privMb = [Math]::Round($p.PrivateMemorySize64 / 1MB, 1)

  $baseCount = -1; $baseFirst = ''
  $c = Items $list
  if ($c) { $baseCount = $c.Count; if ($c.Count) { $baseFirst = RowText $c.Item(0) } }

  $narrowMs = -1; $narrowFirst = ''; $narrowCount = -1
  $broadNote = 'n/a'; $broadFirst = ''
  [void][NativeWin]::SetForegroundWindow($hwnd)
  Start-Sleep -Milliseconds 300

  if ($edit) {
    try { $edit.SetFocus() } catch { }
    Start-Sleep -Milliseconds 200
    $swN = [Diagnostics.Stopwatch]::StartNew()
    [System.Windows.Forms.SendKeys]::SendWait('00123')
    $dl = $swN.ElapsedMilliseconds + 25000
    while ($swN.ElapsedMilliseconds -lt $dl) {
      $c = Items $list
      if ($c -and $c.Count) {
        $f = RowText $c.Item(0)
        if ($f -match '000123') { $narrowMs = $swN.ElapsedMilliseconds; $narrowFirst = $f; $narrowCount = $c.Count; break }
      }
      Start-Sleep -Milliseconds 10
    }
    if ($narrowMs -lt 0) { $c = Items $list; if ($c) { $narrowCount = $c.Count; if ($c.Count) { $narrowFirst = RowText $c.Item(0) } } }

    try { [System.Windows.Forms.SendKeys]::SendWait('^a{BACKSPACE}') } catch { }
    Start-Sleep -Milliseconds 400
    [System.Windows.Forms.SendKeys]::SendWait('e')
    $broadNote = "hung_ms=$(Measure-Hung $hwnd 6000)"
    $c = Items $list
    if ($c -and $c.Count) { $broadFirst = RowText $c.Item(0) }
  } else {
    [System.Windows.Forms.SendKeys]::SendWait('00123')
    $broadNote = "no_accessible_edit hung_ms=$(Measure-Hung $hwnd 6000)"
  }

  $scrollNote = 'no-list-or-no-scrollpattern'
  if ($list) {
    $sp = $null
    try { $sp = $list.GetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern) } catch { }
    if ($sp) {
      $parts = @()
      foreach ($frac in @(0.0, 50.0, 100.0)) {
        $sw3 = [Diagnostics.Stopwatch]::StartNew()
        try {
          $sp.SetScrollPercent([System.Windows.Automation.ScrollPattern]::NoScroll, $frac)
          $c = Items $list
          $first = $(if ($c -and $c.Count) { RowText $c.Item(0) } else { 'none' })
          $last = $(if ($c -and $c.Count) { RowText $c.Item($c.Count - 1) } else { 'none' })
          $parts += ("p{0}:rows={1} first={2} last={3} @{4}ms" -f [int]$frac, $c.Count, $first, $last, $sw3.ElapsedMilliseconds)
        } catch { $parts += ("p{0}:error" -f [int]$frac) }
      }
      $scrollNote = ($parts -join ' | ')
    }
  }

  Stop-Process -Id $p.Id -ErrorAction SilentlyContinue
  Write-Output ("{0} startup_ms={1} uia_tree_ready_ms={2} root={3}[{4}] idle_mem_mb={5} private_mb={6} base_rows={7} base_first={8} narrow_ms={9} narrow_first={10} narrow_rows={11} broad[{12}] broad_first={13} scroll[{14}]" -f `
    $tag, $startup, $treeMs, $rootCT, $rootName, $memMb, $privMb, $baseCount, $baseFirst, $narrowMs, $narrowFirst, $narrowCount, $broadNote, $broadFirst, $scrollNote)
  Start-Sleep -Seconds 3
}
