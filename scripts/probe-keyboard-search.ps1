# Run explicitly on a Windows desktop after building the release native_keyboard_search example.
# The probe uses in-memory preferences. Only its own foreground window receives input.
param([int]$Rounds = 2)
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'This native probe requires Windows x64.' }
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$base = Join-Path $workspaceRoot '_build/goal/keyboard'
New-Item -ItemType Directory -Path $base -Force | Out-Null
$env:TEMP = Join-Path $workspaceRoot '_build/tmp'
$env:TMP = $env:TEMP
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class KeyboardNativeProbe {
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
 }
 [DllImport("user32.dll",SetLastError=true)] public static extern uint SendInput(uint count,Input[] inputs,int size);
 public static void Click(IntPtr h,int x,int y) {
   var p=new IntPtr((y<<16)|(x&65535));
   SendMessage(h,0x200,IntPtr.Zero,p);
   SendMessage(h,0x201,new IntPtr(1),p);
   SendMessage(h,0x202,IntPtr.Zero,p);
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
$process = Start-Process -FilePath (Join-Path $base '../../release/examples/native_keyboard_search.exe') -ArgumentList '--external-input' -PassThru -WindowStyle Hidden -RedirectStandardError $log -RedirectStandardOutput (Join-Path $base 'native-input.out')
$assertions = 0
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
    for ($index = 0; $index -lt $state.query.Length + 2; $index++) { [KeyboardNativeProbe]::Key($script:hwnd,8) }
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
    Capture 'native-input-before.png'
    [KeyboardNativeProbe]::Click($script:hwnd,[int]($ready.search[0]*$ready.scale),[int]($ready.search[1]*$ready.scale))
    for ($round=0;$round -lt $Rounds;$round++) {
        foreach ($character in 'abcdefghijklmnopqrstuvwxyz0123456789'.ToCharArray()) {
            [KeyboardNativeProbe]::Text($script:hwnd,[string]$character)
            [void](Await-Query ([string]$character))
            [KeyboardNativeProbe]::Key($script:hwnd,8)
            [void](Await-Query '')
        }
    }
    foreach ($query in @('copy','files.copy','Copy files to clipboard','žluťoučký','中文 日本語','العربية עברית','é')) {
        [KeyboardNativeProbe]::Text($script:hwnd,$query)
        $state = Await-Query $query
        if ($query -eq 'Copy files to clipboard' -and $state.rows -ne 1) { throw 'Full copy command label must have exactly one match' }
        if ($query -eq 'žluťoučký' -and $state.rows -ne 0) { throw 'Unknown Unicode query must have zero matches' }
        Clear-Query
    }
    for ($round=0;$round -lt 100;$round++) {
        [KeyboardNativeProbe]::Text($script:hwnd,'copyx')
        for ($index=0;$index -lt 5;$index++) { [KeyboardNativeProbe]::Key($script:hwnd,8) }
    }
    [KeyboardNativeProbe]::Text($script:hwnd,'rapid-barrier')
    [void](Await-Query 'rapid-barrier')
    Clear-Query
    [KeyboardNativeProbe]::Click($script:hwnd,[int]($ready.capture[0]*$ready.scale),[int]($ready.capture[1]*$ready.scale))
    Start-Sleep -Milliseconds 100
    [KeyboardNativeProbe]::Key($script:hwnd,119)
    Start-Sleep -Milliseconds 100
    $state = Read-State
    if (-not $state.capturing -or $state.candidate -ne 'F8' -or $state.query -ne '') { throw "Capture focus state mismatch: $($state | ConvertTo-Json -Compress)" }
    $assertions++
    [KeyboardNativeProbe]::Click($script:hwnd,[int]($ready.search[0]*$ready.scale),[int]($ready.search[1]*$ready.scale))
    [KeyboardNativeProbe]::Text($script:hwnd,'copy')
    [void](Await-Query 'copy')
    Capture 'native-input-after.png'
    Clear-Query
    [KeyboardNativeProbe]::Text($script:hwnd,'keyboard-native-complete')
    if (-not $process.WaitForExit(5000)) { throw 'Completion handshake timed out' }
    if ($process.ExitCode -ne 0) { throw "Probe failed: $($process.ExitCode)" }
    "Native message input passed: $assertions query/focus assertions; $Rounds alphabet/digit rounds; 100 rapid edit/clear rounds."
} finally {
    $process.Refresh()
    if (-not $process.HasExited) { [void]$process.CloseMainWindow(); [void]$process.WaitForExit(3000) }
}
