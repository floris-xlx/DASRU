param([string]$Executable = "$PSScriptRoot/../target/debug/dasru-gui.exe")
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class GuiSmokeNative {
    public delegate bool EnumProc(IntPtr window,IntPtr data);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback,IntPtr data);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window,System.Text.StringBuilder text,int count);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    public static IntPtr FindWindow(uint process) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((window,data)=>{ uint id; GetWindowThreadProcessId(window,out id); var title=new System.Text.StringBuilder(256); GetWindowText(window,title,256); Rect rect; GetWindowRect(window,out rect);
            if(id==process && IsWindowVisible(window) && title.ToString()=="DASRU" && rect.Right-rect.Left>=650 && rect.Bottom-rect.Top>=400) { found=window; return false; } return true;
        },IntPtr.Zero); return found;
    }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window,IntPtr dc,uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr window,uint message,IntPtr wparam,IntPtr lparam,uint flags,uint timeout,out IntPtr result);
}
'@
$evidence = Join-Path $PSScriptRoot '../docs/evidence'
New-Item -ItemType Directory -Force $evidence | Out-Null
$timer = [Diagnostics.Stopwatch]::StartNew()
$gameAtStart=@(Get-Process -Name 'DOOMTheDarkAges' -ErrorAction SilentlyContinue).Count -gt 0
$startInfo=New-Object Diagnostics.ProcessStartInfo
$startInfo.FileName=(Resolve-Path -LiteralPath $Executable).Path
$startInfo.UseShellExecute=$false
$app=[Diagnostics.Process]::Start($startInfo)
$processHandle=$app.Handle

try {
    if (!$app.WaitForInputIdle(15000)) { throw 'GUI did not become input-idle' }
    for ($attempt=0; $attempt -lt 75; $attempt++) {
        $app.Refresh()
        if ($app.HasExited) { throw 'GUI exited before creating its window' }
        $handle=[GuiSmokeNative]::FindWindow($app.Id)
        if ($handle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 200
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No native GUI window' }
    $startup = $timer.Elapsed.TotalMilliseconds
    Start-Sleep -Seconds 3
    $rect = New-Object GuiSmokeNative+Rect
    if (![GuiSmokeNative]::GetWindowRect($handle,[ref]$rect)) { throw 'Window rectangle unavailable' }
    $client = New-Object GuiSmokeNative+Rect
    if (![GuiSmokeNative]::GetClientRect($handle,[ref]$client)) { throw 'Client rectangle unavailable' }
    $logicalWidth=($client.Right-$client.Left)*96.0/[GuiSmokeNative]::GetDpiForWindow($handle)
    if ([Math]::Abs($logicalWidth-720.0) -gt 1.0) { throw "Utility client width was $logicalWidth; expected 720" }
    $bitmap = New-Object Drawing.Bitmap ($rect.Right-$rect.Left),($rect.Bottom-$rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $dc = $graphics.GetHdc()
    try { $captured = [GuiSmokeNative]::PrintWindow($handle,$dc,2) }
    finally { $graphics.ReleaseHdc($dc) }
    try { if ($captured) { $bitmap.Save((Join-Path $evidence 'gui-window.png'),[Drawing.Imaging.ImageFormat]::Png) } }
    finally { $graphics.Dispose(); $bitmap.Dispose() }
    $app.Refresh(); $cpuBefore=$app.TotalProcessorTime.TotalMilliseconds; $memoryBefore=$app.WorkingSet64
    $response = [IntPtr]::Zero
    $latencies = @()
    for ($sample=0; $sample -lt 10; $sample++) {
        $probe=[Diagnostics.Stopwatch]::StartNew()
        if ([GuiSmokeNative]::SendMessageTimeout($handle,0,[IntPtr]::Zero,[IntPtr]::Zero,2,500,[ref]$response) -eq [IntPtr]::Zero) { throw 'GUI stopped responding' }
        $latencies += $probe.Elapsed.TotalMilliseconds
        Start-Sleep -Seconds 1
    }
    $app.Refresh(); $cpuAfter=$app.TotalProcessorTime.TotalMilliseconds; $memoryAfter=$app.WorkingSet64
    $dpi=[GuiSmokeNative]::GetDpiForWindow($handle)
    $close=[Diagnostics.Stopwatch]::StartNew()
    if (![GuiSmokeNative]::PostMessage($handle,16,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'Could not request normal window close' }
    if (!$app.WaitForExit(15000)) { throw 'GUI shutdown exceeded 15 seconds' }
    $report=[ordered]@{success=$true;configuration='Windows native debug build; listener not started';game_running_at_start=$gameAtStart;client_width_logical=$logicalWidth;startup_ms=$startup;dpi=$dpi;sample_seconds=10;cpu_ms=($cpuAfter-$cpuBefore);working_set_before=$memoryBefore;working_set_after=$memoryAfter;maximum_message_response_ms=($latencies|Measure-Object -Maximum).Maximum;shutdown_ms=$close.Elapsed.TotalMilliseconds;exit_code=$app.ExitCode;print_window_succeeded=$captured}
    if ($app.ExitCode -ne 0) { throw "GUI exit code $($app.ExitCode)" }
    $report | ConvertTo-Json | Set-Content (Join-Path $evidence 'gui-smoke.json')
    $report | ConvertTo-Json
} catch {
    @{success=$false;error=$_.Exception.Message;game_running_at_start=$gameAtStart} | ConvertTo-Json | Set-Content (Join-Path $evidence 'gui-smoke.json')
    throw
} finally {
    $app.Refresh()
    if (!$app.HasExited) { Stop-Process -Id $app.Id }
    $app.Dispose()
}
