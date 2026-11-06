# Desktop architecture

The Windows-only native `dasru-gui` executable consumes the DASRU library directly.
`desktop-ui` enables pinned egui/eframe 0.37.0; eframe/Glow remain Windows-only.
Default Linux builds do not compile UI dependencies. All-feature Linux builds test
headless egui rendering and the platform-independent controllers.

## Presentation - revised PR #10 direction

A single fixed-width, two-column utility replaces sidebar navigation. Primary
process, build, binding and listener controls remain in one window. Settings,
Help and Diagnostics use separate egui dialog windows over the utility. Dialogs
are presentation state only; opening or closing one does not reset monitoring or
the listener. Auxiliary dialogs are not new OS windows or subprocesses.
Legacy window widths are normalized on load without discarding other preferences
or rewriting their file until a validated save. See gui-design.md for the current
layout contract, which supersedes the initial sidebar specification.

## Ownership and concurrency

`DesktopApp` renders projections and sends typed controller requests. It never
enumerates processes, hashes executables, reads game memory, or monitors keyboards.
`Controller` owns a monitor thread and `HotkeyRuntime`; `SettingsWriter` owns a
separate preferences thread. Close invalidates monitor generations, signals
cancellation, joins hotkey and monitor owners, and flushes final valid preferences.
No child CLI process is launched. No modification session is created.

The monitor has one coalesced refresh slot and one support-export slot, rather
than an unbounded update queue. Snapshots are latest values behind short locks;
blocking operations happen outside those locks. Panics become structured codes.
The preferences queue has capacity one and final shutdown flushes the last value.

## Evidence and freshness

`Observation::from_probe` projects sanitized fields and rejects contradictory
exact-match evidence. Capabilities come from `CapabilityEvidence::from_probe` and
the existing evaluator. Connection and executable verification are distinct.
Progress invalidates the previous observation before expensive reverification.
Generations reject old responses and all responses after shutdown. Worker errors
never preserve a Verified connection. Expired observations are shown as stale.

`ProbeService` caches only exact/unknown-build projections, keyed by PID, process
creation time, path, module identity, file size, creation time and modification
time. Every cache lookup first performs pinned process discovery and liveness
recheck. Cache results expire after 30 seconds. Changed or unavailable keys force
full core verification. Metadata cannot prove the absence of hostile rewrites
that restore timestamps; this cache never authorizes a game operation.

## Hotkeys

The CLI adapter and threaded runtime share `windows::session::run_session`. Typed state,
observation, failure and diagnostic-summary events replace internal terminal
output. CLI formatting remains in the adapter. Focus generations, focus rechecks,
keyboard resets, pinned identity checks and Raw Input registration stay in the
existing backend and manager. The GUI supports one observation binding and never
starts it automatically. A duplicate start is rejected. Stop is requested without
blocking a frame; the owner joins on shutdown. Cleanup failures remain failures.

Executable verification checks cancellation before opening and between 64 KiB
hash chunks. OS process/file calls and support export can still delay shutdown;
there is no forced thread termination or hard wall-clock guarantee for stalled
OS I/O. Support exports finish using the existing authoritative `export_json`
implementation, including no-overwrite and redaction guarantees.

## Preferences and diagnostics

Preferences use schema version 1 in `%LOCALAPPDATA%/DASRU/preferences.json`.
Only theme, window geometry, text scale, observation binding and bounded refresh
interval are persisted. Corrupt/unknown-schema files are preserved, defaults are
used, and saving is disabled for that launch. External file changes are rejected.
Writes use a same-directory temporary file, sync, and atomic replacement.
Windows uses `MoveFileExW`; Unix test files use mode 0600. Monitor geometry is
clamped when restoring a native window. No process IDs or executable paths persist.

Diagnostics only render projected fields and static error codes. Support export
uses a user-entered destination path; no overwrite prompt grants replacement.
The path selector is an input, not a native browse dialog.

The Raw Input event queue has capacity 256 and uses nonblocking sends. Overflow invalidates queued chord generations and surfaces HOTKEY_EVENT_QUEUE_OVERFLOW as a terminal runtime failure. The Windows message loop never waits for event-queue capacity.
