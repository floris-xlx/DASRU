[CmdletBinding()]
param([string]$Executable,[switch]$RestartGame)
$ErrorActionPreference='Stop'
$repository=Split-Path $PSScriptRoot -Parent
if(!$Executable){$Executable=Join-Path $repository 'target/release/dasru-gui.exe'}
if(Get-Process dasru-gui -ErrorAction SilentlyContinue){throw 'Preserve the existing GUI session'}
$game=Get-Process DOOMTheDarkAges -ErrorAction Stop | Select-Object -First 1
$gameHandle=$game.Handle
$originalGameStarted=$game.StartTime
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class GuiControlFocus {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window,out uint id);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window,int command);
    public static uint ForegroundProcess() { uint id; GetWindowThreadProcessId(GetForegroundWindow(),out id); return id; }
}
'@
$evidence=Join-Path $repository ('docs/evidence/gui-controls-'+[DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ'))
New-Item -ItemType Directory -Path $evidence | Out-Null
$scratch=Join-Path $env:TEMP ('dasru-controls-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$trace=Join-Path $evidence 'shutdown.log'
$start=New-Object Diagnostics.ProcessStartInfo
$start.FileName=(Resolve-Path -LiteralPath $Executable).Path
$start.UseShellExecute=$false
$start.EnvironmentVariables['LOCALAPPDATA']=$scratch
$start.Arguments='--shutdown-trace "'+$trace+'"'
$result=[ordered]@{
    source_head=(git -C $repository rev-parse HEAD)
    source_tracked_dirty=[bool](git -C $repository status --porcelain --untracked-files=no)
    binary_sha256=(Get-FileHash -LiteralPath $start.FileName -Algorithm SHA256).Hash
    physical_key_test_performed=$false
    generated_keyboard_events=0
    stages=@()
}
$app=[Diagnostics.Process]::Start($start)
$processHandle=$app.Handle
$stage='window initialization'
function Find-Control([string]$name,$type) {
    $items=$script:root.FindAll([Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty,$name))
    foreach($item in $items){if($item.Current.ControlType -eq $type){return $item}}
    return $null
}
function Read-Value([string]$name) {
    $item=Find-Control $name ([Windows.Automation.ControlType]::Edit)
    if(!$item){return $null}
    return ([Windows.Automation.ValuePattern]$item.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)).Current.Value
}
function Invoke-Control([string]$name) {
    $button=Find-Control $name ([Windows.Automation.ControlType]::Button)
    if(!$button -or !$button.Current.IsEnabled){throw 'GUI_CONTROL_UNAVAILABLE'}
    $pattern=$null
    if($button.TryGetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern,[ref]$pattern)){
        ([Windows.Automation.ExpandCollapsePattern]$pattern).Expand()
    } else {
        ([Windows.Automation.InvokePattern]$button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)).Invoke()
    }
}
function Wait-Until([scriptblock]$condition,[int]$seconds=10) {
    $deadline=[DateTime]::UtcNow.AddSeconds($seconds)
    do {if(& $condition){return}; Start-Sleep -Milliseconds 100} while([DateTime]::UtcNow -lt $deadline)
    throw 'GUI_STATE_TIMEOUT'
}
function Record-Stage([string]$name) {
    $script:result.stages+=@{stage=$name;state=(Read-Value 'Hotkeys');accepted=(Read-Value 'Accepted');foreground_is_game=([GuiControlFocus]::ForegroundProcess() -eq $game.Id)}
}
function Focus-Window([IntPtr]$window) {
    if([GuiControlFocus]::IsIconic($window)){$null=[GuiControlFocus]::ShowWindowAsync($window,9)}
    $null=[GuiControlFocus]::SetForegroundWindow($window)
    if([GuiControlFocus]::GetForegroundWindow() -ne $window){
        try {[Windows.Automation.AutomationElement]::FromHandle($window).SetFocus()} catch { }
    }
}
try {
    if(!$app.WaitForInputIdle(15000)){throw 'GUI_IDLE_TIMEOUT'}
    Start-Sleep -Seconds 2
    $app.Refresh()
    if($app.MainWindowTitle -ne 'DASRU'){throw 'GUI_WINDOW_NOT_FOUND'}
    $script:root=[Windows.Automation.AutomationElement]::FromHandle($app.MainWindowHandle)
    $null=Find-Control 'Start listener' ([Windows.Automation.ControlType]::Button)
    Wait-Until {(Read-Value 'Connection') -eq 'Verified'}
    $result['build']=Read-Value 'Build'
    Focus-Window $app.MainWindowHandle
    if([GuiControlFocus]::ForegroundProcess() -ne $app.Id){throw 'GUI_FOCUS_NOT_GRANTED'}
    $stage='expand observation counters'
    Invoke-Control 'More hotkeys / advanced'
    Wait-Until {$null -ne (Read-Value 'Accepted')}
    $stage='start listener outside game'
    Invoke-Control 'Start listener'
    Wait-Until {(Read-Value 'Hotkeys') -eq 'Inactive'}
    $binding=Find-Control 'Observation shortcut' ([Windows.Automation.ControlType]::Edit)
    $result['binding_disabled_while_listening']=!$binding.Current.IsEnabled
    $result['binding_read_only_while_listening']=([Windows.Automation.ValuePattern]$binding.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)).Current.IsReadOnly
    $result['duplicate_start_disabled']=!(Find-Control 'Start listener' ([Windows.Automation.ControlType]::Button)).Current.IsEnabled
    Record-Stage $stage
    $stage='game foreground'
    $game.Refresh()
    Focus-Window $game.MainWindowHandle
    $result['game_focus_granted']=([GuiControlFocus]::ForegroundProcess() -eq $game.Id)
    if($result.game_focus_granted){Wait-Until {(Read-Value 'Hotkeys') -eq 'Active'}; Record-Stage $stage}
    $stage='game foreground loss'
    Focus-Window $app.MainWindowHandle
    $result['gui_focus_restored']=([GuiControlFocus]::ForegroundProcess() -eq $app.Id)
    if($result.gui_focus_restored){Wait-Until {(Read-Value 'Hotkeys') -eq 'Inactive'}; Record-Stage $stage}
    $stage='stop listener through GUI'
    Invoke-Control 'Stop listener'
    Wait-Until {(Read-Value 'Hotkeys') -eq 'Disabled' -and !(Find-Control 'Stop listener' ([Windows.Automation.ControlType]::Button)).Current.IsEnabled}
    $result['stop_disabled_after_stop']=$true
    Record-Stage $stage
    if($RestartGame){
        $stage='exit game while GUI listener is running'
        Invoke-Control 'Start listener'
        Wait-Until {(Read-Value 'Hotkeys') -in @('Inactive','Active')}
        if(!$game.CloseMainWindow() -or !$game.WaitForExit(30000)){throw 'GAME_EXIT_NOT_OBSERVED'}
        Wait-Until {(Read-Value 'Connection') -eq 'NotRunning' -and !(Find-Control 'Stop listener' ([Windows.Automation.ControlType]::Button)).Current.IsEnabled}
        $result['listener_terminal_after_game_exit']=(Read-Value 'Hotkeys') -in @('Error','Disabled')
        $result['start_disabled_without_game']=!(Find-Control 'Start listener' ([Windows.Automation.ControlType]::Button)).Current.IsEnabled
        Record-Stage $stage
        $stage='restart game and require fresh verification'
        Start-Process -FilePath 'steam://rungameid/3017860'
        Wait-Until {
            $candidate=Get-Process DOOMTheDarkAges -ErrorAction SilentlyContinue | Select-Object -First 1
            $candidate -and $candidate.MainWindowHandle -ne [IntPtr]::Zero
        } 30
        $oldGame=$game
        $game=Get-Process DOOMTheDarkAges -ErrorAction Stop | Select-Object -First 1
        $gameHandle=$game.Handle
        $result['replacement_identity_changed']=$game.StartTime -ne $originalGameStarted -or $game.Id -ne $oldGame.Id
        $oldGame.Dispose()
        Focus-Window $app.MainWindowHandle
        Wait-Until {(Read-Value 'Connection') -eq 'Verified'} 30
        $result['replacement_verified_by_gui']=$true
        $result['old_listener_did_not_reactivate']=!(Find-Control 'Stop listener' ([Windows.Automation.ControlType]::Button)).Current.IsEnabled
        Record-Stage $stage
    }
    $stage='restart listener through GUI'
    Invoke-Control 'Start listener'
    Wait-Until {(Read-Value 'Hotkeys') -in @('Inactive','Active')}
    $game.Refresh()
    Focus-Window $game.MainWindowHandle
    if([GuiControlFocus]::ForegroundProcess() -eq $game.Id){Wait-Until {(Read-Value 'Hotkeys') -eq 'Active'}}
    $result['listener_state_at_close']=Read-Value 'Hotkeys'
    $stage='close GUI with listener running'
    $close=[Diagnostics.Stopwatch]::StartNew()
    $null=$app.CloseMainWindow()
    $result['gui_process_exited']=$app.WaitForExit(15000)
    $close.Stop()
    $result['close_ms']=$close.Elapsed.TotalMilliseconds
    if($result.gui_process_exited){$result['exit_code']=$app.ExitCode}
    $phases=Get-Content -LiteralPath $trace -Raw
    $result['cleanup_phases_succeeded']=$phases.Contains('EFRAME_RETURNED') -and !($phases -match 'success=false')
    foreach($phase in @('SHUTDOWN_REQUESTED','MONITOR_CANCEL_REQUESTED','HOTKEY_STOP_REQUESTED','HOTKEY_STOP_COMPLETED','MONITOR_JOIN_COMPLETED','SETTINGS_FLUSH_COMPLETED','APP_SHUTDOWN_COMPLETED','EFRAME_RETURNED')){
        if(!$phases.Contains($phase)){$result['cleanup_phases_succeeded']=$false}
    }
    $result['foreground_transitions_observed']=$result.game_focus_granted -and $result.gui_focus_restored
    $result['active_listener_close_observed']=$result.listener_state_at_close -eq 'Active'
    $result['test_passed']=$result.binding_disabled_while_listening -and $result.binding_read_only_while_listening -and $result.duplicate_start_disabled -and $result.gui_process_exited -and $result.exit_code -eq 0 -and $result.cleanup_phases_succeeded -and $result.foreground_transitions_observed -and $result.active_listener_close_observed
    if($RestartGame){$result['test_passed']=$result.test_passed -and $result.listener_terminal_after_game_exit -and $result.start_disabled_without_game -and $result.replacement_identity_changed -and $result.replacement_verified_by_gui -and $result.old_listener_did_not_reactivate}
} catch {
    $result['test_passed']=$false
    $result['failure_stage']=$stage
    $result['failure_type']=$_.Exception.GetType().Name
    if($script:root){
        try {
            $result['observed_connection_at_failure']=Read-Value 'Connection'
            $result['observed_listener_state_at_failure']=Read-Value 'Hotkeys'
            $texts=$script:root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
            $codes=@(); foreach($text in $texts){if($text.Current.Name -match '^HOTKEY_[A-Z_]+$'){$codes += $text.Current.Name}}
            $result['listener_error_codes']=$codes | Select-Object -Unique
        } catch { $result['failure_snapshot_available']=$false }
    }
    throw
} finally {
    $app.Refresh()
    if(!$app.HasExited){
        $null=$app.CloseMainWindow()
        if(!$app.WaitForExit(15000)){Write-Warning 'GUI remains live; preserve it for inspection.'}
    }
    $result | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $evidence 'controls.json')
    $result | ConvertTo-Json -Depth 6
    $app.Dispose()
    $game.Dispose()
}
if(!$result.test_passed){throw 'GUI_CONTROL_ACCEPTANCE_FAILED'}
