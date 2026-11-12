# Run explicitly on a Windows desktop after building the release native_keyboard_search example.
# The probe uses in-memory preferences. Only its own foreground window receives input.
param(
    [int]$Rounds = 2,
    [ValidateSet('standard', 'dynamic', 'compact', 'large')][string]$Fixture = 'standard',
    [ValidateSet('SendInput', 'Messages')][string]$InputMode = 'SendInput'
)
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'This native probe requires Windows x64.' }
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$base = Join-Path $workspaceRoot "_build/goal/keyboard-current/$Fixture-$InputMode"
New-Item -ItemType Directory -Path $base -Force | Out-Null
$env:TEMP = Join-Path $workspaceRoot '_build/tmp'
$env:TMP = $env:TEMP
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class KeyboardNativeProbe {
 public static uint ExpectedProcessId;
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [StructLayout(LayoutKind.Sequential)] public struct Point {public int x,y;}
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
 [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h,uint flags);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
 [DllImport("user32.dll")] public static extern short VkKeyScanW(char value);
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags);
 [StructLayout(LayoutKind.Sequential)] public struct Rect {public int left,top,right,bottom;}
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int command);
 [StructLayout(LayoutKind.Explicit,Size=40)] public struct Input {
   [FieldOffset(0)] public uint type;
   [FieldOffset(8)] public ushort vk;
   [FieldOffset(10)] public ushort scan;
   [FieldOffset(12)] public uint flags;
   [FieldOffset(20)] public uint mouseFlags;
 }
 [DllImport("user32.dll",SetLastError=true)] public static extern uint SendInput(uint count,Input[] inputs,int size);
 public static void Click(IntPtr h,int x,int y) {
   var p=new IntPtr((y<<16)|(x&65535));
   SendMessage(h,0x200,IntPtr.Zero,p);
   SendMessage(h,0x201,new IntPtr(1),p);
   SendMessage(h,0x202,IntPtr.Zero,p);
 }
 public static void Move(IntPtr h,int x,int y) {
   RequireOwnedWindow(h);
   var p=new Point{x=x,y=y}; ClientToScreen(h,ref p);
   if(GetAncestor(WindowFromPoint(p),2)!=h) throw new Exception("Probe point is covered by another window; refusing cursor input");
   SetCursorPos(p.x,p.y);
 }
 public static void ActivateOwnWindow(IntPtr h,int x,int y) {
   RequireOwnedWindow(h);
   SetWindowPos(h,new IntPtr(-1),0,0,0,0,0x53);
   try {
     Move(h,x,y);
     var inputs=new[]{new Input{type=0,mouseFlags=2},new Input{type=0,mouseFlags=4}};
     if(SendInput(2,inputs,40)!=2) throw new Exception("Owned-window activation click failed");
     System.Threading.Thread.Sleep(100);
   } finally {SetWindowPos(h,new IntPtr(-2),0,0,0,0,0x53);}
 }
 private static void RequireOwnedWindow(IntPtr h) {
   uint pid; GetWindowThreadProcessId(h,out pid);
   if(pid!=ExpectedProcessId) throw new Exception("Refusing message input to an unowned window");
 }
 public static void MessageText(IntPtr h,string value) {
   RequireOwnedWindow(h);
   foreach(char unit in value) {
     int mapping=VkKeyScanW(unit);
     int vk=mapping<0 ? 0xe7 : mapping&255;
     SendMessage(h,0x100,new IntPtr(vk),new IntPtr(1));
     SendMessage(h,0x102,new IntPtr(unit),new IntPtr(1));
     SendMessage(h,0x101,new IntPtr(vk),new IntPtr(0xc0000001));
   }
 }
 public static void MessageKey(IntPtr h,int value) {
   RequireOwnedWindow(h);
   SendMessage(h,0x100,new IntPtr(value),new IntPtr(1));
   SendMessage(h,0x101,new IntPtr(value),new IntPtr(0xc0000001));
 }
 public static void Key(IntPtr h,int value) {
   if(GetForegroundWindow()!=h) throw new Exception("Probe is not foreground; refusing key input");
   var inputs=new[]{new Input{type=1,vk=(ushort)value},new Input{type=1,vk=(ushort)value,flags=2}};
   if(SendInput(2,inputs,40)!=2) throw new Exception("SendInput key failed");
 }
 public static void Text(IntPtr h,string value) {
   if(GetForegroundWindow()!=h) {
     ShowWindow(h,5);
     SetForegroundWindow(h);
     System.Threading.Thread.Sleep(50);
   }
   if(GetForegroundWindow()!=h) throw new Exception("Probe is not foreground; refusing text input");
   var inputs=new Input[value.Length*2];
   for(int i=0;i<value.Length;i++) {
     inputs[i*2]=new Input{type=1,scan=value[i],flags=4};
     inputs[i*2+1]=new Input{type=1,scan=value[i],flags=6};
   }
   if(SendInput((uint)inputs.Length,inputs,40)!=inputs.Length) throw new Exception("SendInput failed");
 }
}
'@
[void][KeyboardNativeProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))
$env:RUST_BACKTRACE = 'full'
$env:WGPU_BACKEND = 'dx12'
$log = Join-Path $base 'native-input.err'
$process = Start-Process -FilePath (Join-Path $workspaceRoot '_build/release/examples/native_keyboard_search.exe') -ArgumentList @('--external-input', "--fixture=$Fixture") -WorkingDirectory $workspaceRoot -PassThru -WindowStyle Hidden -RedirectStandardError $log -RedirectStandardOutput (Join-Path $base 'native-input.out')
$assertions = 0
function Send-ProbeText([string]$Value) {
    if ($InputMode -eq 'Messages') { [KeyboardNativeProbe]::MessageText($script:hwnd,$Value) }
    else { [KeyboardNativeProbe]::Text($script:hwnd,$Value) }
}
function Send-ProbeKey([int]$Value) {
    if ($InputMode -eq 'Messages') { [KeyboardNativeProbe]::MessageKey($script:hwnd,$Value) }
    else { [KeyboardNativeProbe]::Key($script:hwnd,$Value) }
}
function Read-State {
    $line = Get-Content -LiteralPath $log | Where-Object { $_.StartsWith('external-state=') } | Select-Object -Last 1
    if ($line) { return $line.Substring(15) | ConvertFrom-Json }
    return $null
}
function Await-Query([string]$Expected) {
    $deadline = [DateTime]::UtcNow.AddSeconds(3)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw "Probe exited $($process.ExitCode), expected query '$Expected': $(Get-Content -LiteralPath $log -Raw)" }
        $state = Read-State
        if ($null -ne $state -and $state.query -ceq $Expected) { $script:assertions++; return $state }
        Start-Sleep -Milliseconds 20
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Query mismatch: expected '$Expected', got '$($state.query)'"
}
function Clear-Query {
    $state = Read-State
    for ($index = 0; $index -lt $state.query.Length + 2; $index++) { Send-ProbeKey 8 }
    [void](Await-Query '')
}
function Capture([string]$Name) {
    $rect = [KeyboardNativeProbe+Rect]::new()
    [void][KeyboardNativeProbe]::GetClientRect($script:hwnd,[ref]$rect)
    $bitmap = [Drawing.Bitmap]::new($rect.right,$rect.bottom)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $dc = $graphics.GetHdc()
    try { [void][KeyboardNativeProbe]::PrintWindow($script:hwnd,$dc,3) }
    finally { $graphics.ReleaseHdc($dc); $graphics.Dispose() }
    try { $bitmap.Save((Join-Path $base $Name),[Drawing.Imaging.ImageFormat]::Png) }
    finally { $bitmap.Dispose() }
}
try {
    $ready = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        Start-Sleep -Milliseconds 50
        $process.Refresh()
        if ($process.HasExited) { throw "Probe startup failed: $(Get-Content -LiteralPath $log -Raw)" }
        $line = Get-Content -LiteralPath $log | Where-Object { $_.StartsWith('external-ready=') } | Select-Object -Last 1
        if ($line) { $ready = $line.Substring(15) | ConvertFrom-Json }
    } while ($null -eq $ready -and [DateTime]::UtcNow -lt $deadline)
    if ($null -eq $ready) { throw 'Probe never became ready' }
    $script:hwnd = $process.MainWindowHandle
    if ($script:hwnd -eq 0) { throw 'No native window handle' }
    [KeyboardNativeProbe]::ExpectedProcessId = $process.Id
    [void][KeyboardNativeProbe]::ShowWindow($script:hwnd,5)
    [void][KeyboardNativeProbe]::SetForegroundWindow($script:hwnd)
    Start-Sleep -Milliseconds 100
    Capture 'native-input-before.png'
    if ($null -ne $ready.search) {
        [KeyboardNativeProbe]::ActivateOwnWindow($script:hwnd,[int]($ready.search[0]*$ready.scale),[int]($ready.search[1]*$ready.scale))
        [KeyboardNativeProbe]::Click($script:hwnd,[int]($ready.search[0]*$ready.scale),[int]($ready.search[1]*$ready.scale))
    } else {
        Write-Output 'Search control is clipped: retained search focus established at full size before resize.'
    }
    if ($Fixture -ne 'compact' -and $null -ne $ready.search) {
        # Keep text focus in Search while hovering a disposable command-row tooltip.
        # First 'x' removes the diagnostic rows, exercising popup teardown during filtering.
        [KeyboardNativeProbe]::Move($script:hwnd,[int]($ready.search[0]*$ready.scale),[int](($ready.search[1]+48)*$ready.scale))
        Start-Sleep -Milliseconds 900
        Capture 'native-row-hover.png'
        Send-ProbeText 'x'
        [void](Await-Query 'x')
        Clear-Query
    }
    for ($round=0;$round -lt $Rounds;$round++) {
        foreach ($character in 'abcdefghijklmnopqrstuvwxyz0123456789'.ToCharArray()) {
            Send-ProbeText ([string]$character)
            [void](Await-Query ([string]$character))
            Send-ProbeKey 8
            [void](Await-Query '')
        }
    }
    foreach ($query in @('copy','files.copy','Copy files to clipboard','žluťoučký','中文 日本語','العربية עברית','é')) {
        Send-ProbeText $query
        $state = Await-Query $query
        if ($query -eq 'Copy files to clipboard' -and $state.rows -ne 1) { throw 'Full copy command label must have exactly one match' }
        if ($query -eq 'žluťoučký' -and $state.rows -ne 0) { throw 'Unknown Unicode query must have zero matches' }
        Clear-Query
    }
    for ($round=0;$round -lt 100;$round++) {
        Send-ProbeText 'copyx'
        for ($index=0;$index -lt 5;$index++) { Send-ProbeKey 8 }
    }
    Send-ProbeText 'rapid-barrier'
    [void](Await-Query 'rapid-barrier')
    Clear-Query
    if ($null -ne $ready.capture -and $null -ne $ready.search) {
        [KeyboardNativeProbe]::Click($script:hwnd,[int]($ready.capture[0]*$ready.scale),[int]($ready.capture[1]*$ready.scale))
        Start-Sleep -Milliseconds 100
        Send-ProbeKey 119
        $captureDeadline = [DateTime]::UtcNow.AddSeconds(3)
        do {
            $state = Read-State
            if ($state.capturing -and $state.candidate -eq 'F8' -and $state.query -eq '') { break }
            Start-Sleep -Milliseconds 20
        } while ([DateTime]::UtcNow -lt $captureDeadline)
        if (-not $state.capturing -or $state.candidate -ne 'F8' -or $state.query -ne '') { throw "Capture focus state mismatch: $($state | ConvertTo-Json -Compress)" }
        $assertions++
        [KeyboardNativeProbe]::Click($script:hwnd,[int]($ready.search[0]*$ready.scale),[int]($ready.search[1]*$ready.scale))
    } else { Write-Output 'Capture field is clipped: capture-focus stage skipped for this fixture.' }
    Send-ProbeText 'copy'
    [void](Await-Query 'copy')
    Capture 'native-input-after.png'
    Clear-Query
    Send-ProbeText 'keyboard-native-complete'
    if (-not $process.WaitForExit(5000)) { throw 'Completion handshake timed out' }
    if ($process.ExitCode -ne 0) { throw "Probe failed: $($process.ExitCode)" }
    "Native input passed ($Fixture, $InputMode): $assertions query/focus assertions; $Rounds alphabet/digit rounds; 100 rapid edit/clear rounds."
} finally {
    $process.Refresh()
    if (-not $process.HasExited) { [void]$process.CloseMainWindow(); [void]$process.WaitForExit(3000) }
}
