# PR #10 controlled native acceptance

This is a procedure, not a PASS record. Record observed results in
`pr10-tdd-evidence.md`; physical keys cannot be replaced by injection or CI.
Keep PR #10 draft until every mandatory gate passes.

## Preparation

Use the existing branch and record `git rev-parse HEAD` and the executable SHA-256.
Build on native Windows with Rust 1.95:

```powershell
cargo +1.95.0 build --release --locked --features desktop-ui --bin dasru-gui
Get-FileHash target/release/dasru-gui.exe -Algorithm SHA256
```

Run automated Rust tests through WSL2. Avoid unrelated compilation/disk-heavy jobs
during controlled performance measurements. Note OS, CPU, display resolution,
native scaling, game state, and whether a debugger is attached. Do not attach
paths, raw PIDs, environment values, memory dumps or unrestricted process lists.

## B: physical game and listener

1. Launch the release GUI without TDA. Record NotRunning/NotFound, unavailable
   build and disabled Start. Launch TDA normally; allow verification to complete.
   Record exact registered-build status and the displayed build identifier.
2. Expand More hotkeys / advanced. Record accepted/rejected counters and binding.
   Confirm Ctrl+Alt+F9 validates. Start the listener from the GUI while TDA is
   unfocused: record Inactive. Start again if enabled: no second owner; otherwise
   record the correctly disabled control.
3. Focus TDA. Observe Active using a second view when possible, or retain the
   backend focus/activation diagnostics before switching away. Press the chord
   physically three separate times, releasing all keys between presses. Hold the
   third press for two seconds to test repeat suppression. Accepted count must
   increase by exactly three, including that held press. Typed Action and
   Observation notifications must not count the same press twice.
4. Switch away. Record Inactive. Physically press the same chord once while
   unfocused: accepted count must stay unchanged. Record rejection/last-event
   diagnostics. Refocus TDA and press once: accepted count must increase by one.
5. Stop while unfocused, then stop again if enabled. Record terminal state and
   cleanup errors/counters. Restart and stop across a focus transition.
6. Restart listening and exit TDA normally. Record deactivation and invalidation.
   Restart TDA normally: the replacement identity must undergo new verification;
   the old listener must not authorize it. Restart the GUI listener only after
   fresh verification and confirm one physical observation.
7. Close DASRU with the listener active. Record phase times, terminal process
   status, and absence of the process/worker owner afterward. Repeat close with
   listener inactive, with game absent, after game exit, during an export to a
   new destination, and during a valid preferences save. Never treat a killed
   process as successful cleanup. Preserve any stalled process for investigation.

Use an opt-in new trace file for a manual launch:

```powershell
$trace = Join-Path $env:TEMP ('dasru-shutdown-' + [guid]::NewGuid() + '.log')
& .\target\release\dasru-gui.exe --shutdown-trace $trace
```

Copy the trace only after checking it contains static phases/codes. The trace
refuses existing destinations. A completed app phase does not prove exit until
EFRAME_RETURNED and actual process termination are observed.

## C: native display matrix

For each of 100%, 125%, 150%, 200% Windows display scaling, restart the native app
and capture the actual window. Test practical 1366×768 and 1920×1080 resolutions
where available. Check 720 logical client width, paired panel alignment, disabled
read-only values, unclipped statuses, expanded details, minimum-height scrolling,
footer access, and Settings/Help/Diagnostics closing at text scales 0.8–1.6.
Restore the user's original display configuration afterward.

With two different-DPI monitors, move the window in both directions; close and
restore it on each monitor. Disconnect the saved monitor and relaunch: the title
bar and footer must remain accessible. With one monitor, mark this PENDING.
Headless rendering at equivalent scales is regression evidence only.

## D: release performance

For controlled no-game/inactive-listener measurements:

```powershell
powershell -NoProfile -File scripts/gui-smoke.ps1 -SampleSeconds 900
```

The harness opens a visible release GUI, samples WM_NULL response, process CPU
time and working set once a second, captures its window, and requests normal
WM_CLOSE. Each run creates a separate timestamped evidence directory. It checks
all cleanup phases and exit code; no keyboard events are injected. CPU is a
percentage of one logical core, not divided by the machine's logical CPU count.
Synchronous opt-in trace output adds some close-time measurement overhead.

The unattended harness does not start a listener; its report says so. Do not use
that report to claim active-listener acceptance. For TDA inactive/active steady
modes, collect the same process counters for at least fifteen minutes while an
operator records actual GUI listener state, counters, focus and game transitions.
Record repeated physical start/stop and focus cycles separately. Compare memory
after warmup and across repeated cycles; explain growth rather than declaring
absence of leaks from two samples. Budgets: startup <2s, one-core idle CPU <3%,
working set preferably <150MB, normal response <100ms, close <5s. A stalled OS
call is an unresolved exception for review, not a PASS.

## Failures and evidence

For the final-preferences conflict case, run the smoke harness in a child
PowerShell process whose ProcessStartInfo.EnvironmentVariables["LOCALAPPDATA"]
points to a new isolated temporary directory. Wait until its test-owned DASRU
window is visible (the settings store has then loaded), create
`DASRU/preferences.json` in that temporary directory with known fixture bytes,
and let the harness request ordinary close. Require the fixture to remain exact,
GUI exit code 1, SETTINGS_CHANGED_EXTERNALLY on both failed flush and final app
phases, EFRAME_RETURNED, and no surviving GUI process. Keep the raw smoke report
as success=false and record the expected-failure assertion separately. Do not
redirect or modify the user's actual preferences for this test.

On timeout preserve the trace and process for diagnosis; record the last completed
phase and outstanding owner. Do not forcibly terminate threads or silently detach
workers. Filesystem/process calls and joins cannot promise a hard wall-clock bound.
Use a unique support destination; verify no-overwrite and sanitized report fields.
Record every observed failure and the exact head/binary tested. Leave unavailable
hardware/game modes PENDING and do not extrapolate from hosted Windows tests.
