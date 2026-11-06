[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1,86400)][int]$SampleSeconds = 10,
    [ValidateRange(480,1200)][Nullable[int]]$ClientHeight
)
$ErrorActionPreference = 'Stop'
if (!$Executable) { $Executable = Join-Path $PSScriptRoot '../target/release/dasru-gui.exe' }
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
    [StructLayout(LayoutKind.Sequential)] public struct MonitorInfo { public int Size; public Rect Monitor,Work; public uint Flags; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr window,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern bool GetMonitorInfo(IntPtr monitor,ref MonitorInfo info);
    public static bool GetWorkArea(IntPtr window,out Rect work) {
        var info=new MonitorInfo { Size=Marshal.SizeOf(typeof(MonitorInfo)) };
        bool success=GetMonitorInfo(MonitorFromWindow(window,2),ref info);
        work=info.Work;
        return success;
    }
    public static bool ResizeClient(IntPtr window,int logicalHeight) {
        Rect outer,client,work;
        if(!GetWindowRect(window,out outer) || !GetClientRect(window,out client) || !GetWorkArea(window,out work)) return false;
        int height=(int)Math.Round(logicalHeight*GetDpiForWindow(window)/96.0)+outer.Bottom-outer.Top-client.Bottom+client.Top;
        int width=outer.Right-outer.Left;
        if(height>work.Bottom-work.Top || width>work.Right-work.Left) return false;
        int x=Math.Max(work.Left,Math.Min(outer.Left,work.Right-width));
        int y=Math.Max(work.Top,Math.Min(outer.Top,work.Bottom-height));
        return SetWindowPos(window,IntPtr.Zero,x,y,width,height,0x14);
    }
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window,IntPtr dc,uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr window,uint message,IntPtr wparam,IntPtr lparam,uint flags,uint timeout,out IntPtr result);
}
'@
$runName = 'gui-smoke-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')
$evidence = Join-Path $PSScriptRoot "../docs/evidence/$runName"
New-Item -ItemType Directory $evidence -ErrorAction Stop | Out-Null
$trace = Join-Path (Resolve-Path -LiteralPath $evidence).Path 'shutdown.log'
$timer = [Diagnostics.Stopwatch]::StartNew()
$gameAtStart=@(Get-Process -Name 'DOOMTheDarkAges' -ErrorAction SilentlyContinue).Count -gt 0
$startInfo=New-Object Diagnostics.ProcessStartInfo
$startInfo.FileName=(Resolve-Path -LiteralPath $Executable).Path
$startInfo.UseShellExecute=$false
$startInfo.Arguments='--shutdown-trace "' + $trace + '"'
$binaryHash=(Get-FileHash -LiteralPath $startInfo.FileName -Algorithm SHA256).Hash
$profile=Split-Path (Split-Path $startInfo.FileName -Parent) -Leaf
$sourceHead=git -C "$PSScriptRoot/.." rev-parse HEAD
$trackedDirty=[bool](git -C "$PSScriptRoot/.." status --porcelain --untracked-files=no)
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
    if ($null -ne $ClientHeight) {
        if (![GuiSmokeNative]::ResizeClient($handle,$ClientHeight)) { throw 'Could not resize test GUI client' }
    }
    Start-Sleep -Seconds 3
    $rect = New-Object GuiSmokeNative+Rect
    if (![GuiSmokeNative]::GetWindowRect($handle,[ref]$rect)) { throw 'Window rectangle unavailable' }
    $work=New-Object GuiSmokeNative+Rect
    if (![GuiSmokeNative]::GetWorkArea($handle,[ref]$work)) { throw 'Monitor work area unavailable' }
    if ($rect.Left -lt $work.Left -or $rect.Top -lt $work.Top -or $rect.Right -gt $work.Right -or $rect.Bottom -gt $work.Bottom) { throw 'GUI outer frame exceeds visible work area' }
    $client = New-Object GuiSmokeNative+Rect
    if (![GuiSmokeNative]::GetClientRect($handle,[ref]$client)) { throw 'Client rectangle unavailable' }
    $logicalWidth=($client.Right-$client.Left)*96.0/[GuiSmokeNative]::GetDpiForWindow($handle)
    $logicalHeight=($client.Bottom-$client.Top)*96.0/[GuiSmokeNative]::GetDpiForWindow($handle)
    if ([Math]::Abs($logicalWidth-720.0) -gt 1.0) { throw "Utility client width was $logicalWidth; expected 720" }
    if ($null -ne $ClientHeight -and [Math]::Abs($logicalHeight-$ClientHeight) -gt 1.0) { throw "Utility client height was $logicalHeight; expected $ClientHeight" }
    $bitmap = New-Object Drawing.Bitmap ($rect.Right-$rect.Left),($rect.Bottom-$rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $dc = $graphics.GetHdc()
    try { $captured = [GuiSmokeNative]::PrintWindow($handle,$dc,2) }
    finally { $graphics.ReleaseHdc($dc) }
    try { if ($captured) { $bitmap.Save((Join-Path $evidence 'gui-window.png'),[Drawing.Imaging.ImageFormat]::Png) } }
    finally { $graphics.Dispose(); $bitmap.Dispose() }
    $app.Refresh(); $cpuBefore=$app.TotalProcessorTime.TotalMilliseconds; $memoryBefore=$app.WorkingSet64
    $response = [IntPtr]::Zero
    $samples = New-Object 'Collections.Generic.List[object]'
    $observation=[Diagnostics.Stopwatch]::StartNew()
    for ($sample=0; $sample -lt $SampleSeconds; $sample++) {
        $probe=[Diagnostics.Stopwatch]::StartNew()
        if ([GuiSmokeNative]::SendMessageTimeout($handle,0,[IntPtr]::Zero,[IntPtr]::Zero,2,500,[ref]$response) -eq [IntPtr]::Zero) { throw 'GUI stopped responding' }
        $latency=$probe.Elapsed.TotalMilliseconds
        $app.Refresh()
        $samples.Add([ordered]@{elapsed_ms=$observation.Elapsed.TotalMilliseconds;cpu_ms=$app.TotalProcessorTime.TotalMilliseconds;working_set=$app.WorkingSet64;message_response_ms=$latency})
        Start-Sleep -Seconds 1
    }
    $observedMs=$observation.Elapsed.TotalMilliseconds
    $app.Refresh(); $cpuAfter=$app.TotalProcessorTime.TotalMilliseconds; $memoryAfter=$app.WorkingSet64
    $dpi=[GuiSmokeNative]::GetDpiForWindow($handle)
    $close=[Diagnostics.Stopwatch]::StartNew()
    if (![GuiSmokeNative]::PostMessage($handle,16,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'Could not request normal window close' }
    if (!$app.WaitForExit(15000)) { throw 'GUI shutdown exceeded 15 seconds' }
    $close.Stop()
    if ($app.ExitCode -ne 0) { throw "GUI exit code $($app.ExitCode)" }
    $phases=Get-Content -LiteralPath $trace
    foreach ($phase in @('SHUTDOWN_REQUESTED','HOTKEY_STOP_REQUESTED','HOTKEY_STOP_COMPLETED','MONITOR_CANCEL_REQUESTED','MONITOR_JOIN_COMPLETED','SETTINGS_FLUSH_COMPLETED','APP_SHUTDOWN_COMPLETED','EFRAME_RETURNED')) {
        if (!($phases -match ('phase="' + $phase + '"'))) { throw "Missing shutdown phase: $phase" }
    }
    if ($phases -match 'success=false') { throw 'Cleanup phase reported failure' }
    $samples | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $evidence 'samples.json')
    $report=[ordered]@{success=$true;configuration="Windows native $profile build; listener not started";source_head=$sourceHead;source_tracked_dirty=$trackedDirty;binary_sha256=$binaryHash;game_running_at_start=$gameAtStart;client_width_logical=$logicalWidth;startup_ms=$startup;dpi=$dpi;sample_seconds=$SampleSeconds;observed_ms=$observedMs;cpu_ms=($cpuAfter-$cpuBefore);one_core_cpu_percent=(100.0*($cpuAfter-$cpuBefore)/$observedMs);working_set_before=$memoryBefore;working_set_after=$memoryAfter;maximum_working_set=($samples.working_set|Measure-Object -Maximum).Maximum;maximum_message_response_ms=($samples.message_response_ms|Measure-Object -Maximum).Maximum;shutdown_ms=$close.Elapsed.TotalMilliseconds;exit_code=$app.ExitCode;shutdown_phases_complete=$true;listener_cleanup='not exercised; listener never started';print_window_succeeded=$captured}
    $report['client_height_logical']=$logicalHeight
    $report['requested_client_height']=$ClientHeight
    $report['outer_bounds']=$rect
    $report['work_area']=$work
    $report['entire_window_visible']=$true
    $report | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $evidence 'gui-smoke.json')
    $report | ConvertTo-Json -Depth 4
} catch {
    @{success=$false;error=$_.Exception.Message;game_running_at_start=$gameAtStart;source_head=$sourceHead;binary_sha256=$binaryHash} | ConvertTo-Json | Set-Content (Join-Path $evidence 'gui-smoke.json')
    throw
} finally {
    $app.Refresh()
    # Keep a stalled process available for diagnosis; forced termination is not cleanup evidence.
    if (!$app.HasExited) { Write-Warning 'GUI remains running after failure; inspect and close it manually.' }
    $app.Dispose()
}
