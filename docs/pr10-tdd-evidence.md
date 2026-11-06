# Desktop UI TDD evidence

## Production hardening baseline (2026-11-06)

Fetched origin before changing behavior. Main remains
`4e637752b9241cff7805e6ee9baf72c649557e10`; PR head remains
`7c037af8b653159eefaf700b883833f2aa2ac18c`. Branch is
`feat/desktop-ui-foundation`, checkout clean, PR #10 draft and mergeable.

Fresh baseline: 234 WSL tests passed with the installed stable Rust 1.99.1.
The full output is preserved in the timestamped
`evidence/pr10-hardening-baseline-7c037af-*-wsl-tests.log`, with local paths
redacted. This is new baseline evidence, not new fix coverage. Rust 1.95.0 was
also installed in WSL for the hardening checks. A clean native Windows
`cargo +1.95.0 build --release --locked --features desktop-ui --bin dasru-gui`
passed. The GUI dependencies remain pinned at 0.37.0, MSRV 1.95; locked build
made no manifest or lockfile changes. Baseline release SHA-256:
`FE523F24670D7E90BEADD11D07C936ECD3EF5FF53A2579879E0783B5A9E36E35`.

Baseline hosted runs 42199588921 and 42199582667 passed Windows and Ubuntu,
including Rust 1.95 formatting, warnings-as-errors Clippy, tests and Windows
release GUI build. These do not validate subsequent hardening commits.

The requested Cargo/CI, GUI, hotkey, smoke and architecture/design/evidence
surfaces were inspected. Existing tests cover state projection, freshness,
coalescing, panic conversion, shared dispatch IDs, late observations, atomic
preferences, support redaction/no-overwrite and real Windows registration
cleanup. Missing deterministic fault coverage includes startup completion-channel
failure, outstanding export cancellation, full settings queue, and preserving
cleanup errors beyond the disappearing UI.

### Initial ownership audit - hypotheses, not a timeout explanation

| Owner | Current cancellation and wait | Finding |
|---|---|---|
| Controller | cancel flag + condition-variable wake; synchronous join | Joins outside shared locks, but OS/process/file calls and in-flight export can delay it; queued export status can remain Pending after close |
| HotkeyRuntime | stop flag; synchronous join | Keeps high-level owner, but inner Win32 fallback can still detach its worker |
| WindowsBackend | posted stop message; 3s completion waits | Shutdown early errors retain the handle only until Drop; Drop ignores retry errors. abort_startup joins only on successful completion receipt, detaching on timeout/disconnect |
| SettingsWriter | final bounded-channel send; join | No shared status lock held while waiting; OS write/sync/replace and full queue can delay finish |
| DesktopApp | on_exit -> controller then settings | Cleanup errors only stored on an app about to be destroyed; later settings error can replace earlier cleanup error |
| eframe/Glow | on_exit then painter destruction and event-loop return | Renderer/window teardown occurs after app cleanup; app-completed markers alone cannot prove process exit |
| Smoke harness | WM_CLOSE; process wait; failure cleanup | Existing fixed filenames overwrite history; no phase trace; failed tests forcibly clean up their owned process, which is not evidence of graceful cleanup |

Cancellation is cooperative. No hard wall-clock bound exists for uninterruptible
OS calls or resource-owner joins. The historical 15-second timeout is still
unexplained; no Gate A PASS is claimed from this audit.

| Gate | Baseline status |
|---|---|
| A Shutdown | FAIL - known ownership/error-reporting gaps; historical timeout unresolved |
| B Physical GUI/TDA | PENDING - no observed GUI physical session; TDA not running at preflight |
| C Native displays | PENDING - historical 100% capture/headless scaling do not establish all native scenarios |
| D Sustained release performance | PENDING - no fifteen-minute release series yet |
| E Reliability review | PENDING - baseline regressions pass, critical shutdown findings open |
| F Distribution/final CI | PENDING - baseline CI green, notices/license/final-head review still required |

## Current presentation revision: compact DESRU-style utility

The user's revised direction supersedes the sidebar/page specification. The
current design contract is gui-design.md: fixed 720-point width, paired utility
panels, one primary window, expandable hotkey details, and separate Settings,
Help and Diagnostics dialogs. Rust services and safeguards remain authoritative.
Earlier sidebar screenshots and measurements below are historical, not current
visual acceptance evidence.

Observed red/green cycles for this revision:

- Primary-panel test failed with missing KEYBINDS and painted Dashboard/sidebar
  labels; now all seven panel headers and three footer buttons are present.
- Default geometry failed [1000, 680] versus [720, 700]; compact defaults now pass.
- Dialog tests initially failed compilation with missing Dialog/open/close API;
  open/close and 100-200% headless render tests now pass while the utility remains.
- Legacy-width migration failed [1000, 680] versus [720, 680]; normalized load now
  preserves binding/theme and leaves the original file untouched until save.
- Dialog text-scale test failed because the new window inherited unscaled text;
  shared context styling now applies the preference to dialog instructions.
- Native screenshot revealed clipped status fields. The painted-text assertion
  failed with NotRunning is clipped; shared minimum input sizing now passes.
- Panel alignment, footer visibility and idle repaint-delay checks pass. The idle
  check was a regression safeguard, not a claimed red/green failure cycle.

Initial layout-revision local checks: 202 WSL all-target/all-feature tests passed, Windows and WSL
Clippy passed with warnings denied, formatting passed, and Windows Rust 1.95
all-target/all-feature check passed. New hosted CI is pending after this push;
the prior green runs below do not by themselves validate this layout revision.

Current native Windows evidence: evidence/gui-window.png shows the compact main
utility; evidence/gui-compact-diagnostics.png shows the real Diagnostics dialog.
The smoke test asserts a 720-logical-pixel client width. Latest debug/no-game
sample at 96 DPI: visible startup 403.7 ms, 234.4 ms process CPU over ten seconds,
working set 77,152,256 -> 76,828,672 bytes, maximum message response 6.28 ms,
normal close 90.5 ms, exit code 0. See evidence/gui-smoke.json. These short samples
are not sustained or release idle-performance acceptance.

FPS limiting, freescroll, resolution scaling and trainer controls remain absent
or explicitly unavailable. Original/custom DESRU fonts and source were not copied.
Physical TDA hotkeys, native high-DPI/multi-monitor behavior, sustained performance
and the previously recorded isolated close-timeout investigation remain open.

## Merge reconciliation with current main

Main advanced to 4e63775 (merged configurable-hotkeys PR #9). Its config/dispatch
modules, CLI arguments, tests and original pr9-tdd-evidence.md are preserved. This
GUI evidence is renamed pr10-tdd-evidence.md to avoid overwriting that history.
The Windows session now accepts ConfiguredBinding and uses ActionDispatcher with
a typed runtime sink. IDs, generation checks, shutdown and CLI output ordering
are retained; GUI observation counts are not doubled by typed Action events.

RED: bridge test could not compile without RuntimeActionSink / RuntimeEvent::Action.
GREEN: configured ID forwarding, stale-generation rejection, shutdown fencing and
single accepted-count tests pass. The earlier Windows merge check also correctly
failed on the stale ConsoleActionSink interface, exposing the seam before repair.
A second regression test emitted an in-flight observation after Stop: RED counted
1 instead of 0; GREEN drops action/observation callbacks after cancellation while
still accepting terminal state and cleanup-error events.

Final reconciled local results: 234 WSL all-target/all-feature tests passed,
Windows and WSL all-feature Clippy passed with warnings denied, Rust 1.95 check
passed and formatting passed. Hosted CI must validate the reconciled head after
push; earlier source jobs are historical.

## Historical foundation implementation evidence

Base: 378063005c3669426b87e6d5d1343b92f8ce02eb.
Branch: feat/desktop-ui-foundation. Draft PR #10 (#9 already existed).

## Observed Red -> Green cycles

| Package | Observed RED | GREEN |
|---|---|---|
| WP-01 | Missing desktop-ui feature and dasru::gui/egui imports | Native executable builds with pinned 0.37.0, Glow, Rust 1.95; CLI preserved |
| WP-02 | gui_state unresolved model module | Disconnected/exact/unknown/ambiguous/access/identity/error states, stale results and shutdown invalidation pass |
| WP-02 regression | worker_error_never_keeps_verified_connection returned Verified instead of Error | Any worker failure now overrides old verification; test passes |
| WP-03 | gui_controller unresolved controller module | Slow-service nonblocking calls, 100-request coalescing, cancellation, idempotent shutdown and panic projection pass |
| WP-03 cache | gui_monitor unresolved services module | Unchanged identity avoids repeated verification, changed identity re-verifies, cancellation avoids new work |
| WP-03 progress | process_change_clears_old_verification_while_hashing missing progress method | Verifying clears old observation and ignores old-generation progress |
| WP-04 | Missing theme projection | All four screens render in headless tests at 1, 1.25, 1.5 and 2 pixels per point; navigation survives frames |
| WP-04 regression | dark_theme_tokens_apply_even_when_windows_prefers_light returned blue instead of red | Explicit theme selection applies shared tokens even on Windows light preference |
| WP-05 | gui_hotkeys missing runtime API | Duplicate start rejection, typed accepted-event snapshots, joined stop and owner-drop tests pass; existing manager/focus-generation tests retained |
| WP-06 | gui_support missing export methods/status | Background export preserves redaction schema and refuses existing destination without modifying its bytes |
| WP-07 | gui_settings missing settings API, then missing SettingsWriter | Defaults, schema/binding/interval/scale validation, corrupt-file preservation, round trip, geometry clamp and final background flush pass |
| WP-08 hashing | Missing cancellable inspection methods | Pre-cancel avoids opening a file; deterministic cancellation occurs between hash chunks |
| WP-08 cleanup | cleanup_failure_is_not_reported_as_success returned Ok | Shutdown failure now remains an error; regression passes |

The initial bootstrap test was accidentally invoked using native Windows Cargo;
all subsequent local `cargo test` executions use WSL2. The first headless render
also failed because egui 0.37 requires explicit texture-delta handling; the harness
now clears those deltas. Seven pre-existing collapsible-if lints were mechanically
updated after the reviewed MSRV bump. No tests are claimed for unexercised hardware.

## Local automated verification

- WSL2 `cargo test --locked --all-targets --all-features`: 197 passed, 0 failed across 21 test binaries (Windows-only tests excluded by target).
- Windows `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- Windows `cargo +1.95.0 check --locked --all-targets --all-features`: passed during implementation; final recheck recorded with delivery.
- Native Windows debug GUI build: passed.
- Existing CLI tests, capability tests, identity tests, session tests, hotkey manager/focus replay tests and privacy-safe support export regressions remain included.

## Native Windows evidence

`scripts/gui-smoke.ps1` launches the actual GUI, finds the visible DASRU window,
captures only that window, samples message responsiveness/CPU/working set, sends
normal close and checks clean process exit. This uses Windows UI messages only
against the test-owned GUI, never game input. It does not simulate a physical
shortcut or test gameplay behavior. TDA was running throughout the recorded test.

The first smoke attempt incorrectly selected a transient helper window and its
8-second close measurement and black capture are invalid. The harness was fixed
to match title, visibility and minimum dimensions. A navigation-message experiment
was removed because posted button messages did not reliably select coordinates.
Only the final inspected screenshot and corresponding JSON are retained.

The earlier game-running debug sample at 96 DPI showed actual Verified process
status and registered build `steam-25755602`; retain its inspected screenshot as
`evidence/gui-game-running.png`. That capture predates the final layout rows.

Two final-source debug runs with no game, at 96 DPI, passed visible-window,
message responsiveness and normal-close checks. Startup was 417.4 / 382.8 ms;
10-second CPU samples consumed 187.5 / 281.25 ms; maximum message response was
5.94 / 5.73 ms; normal shutdown took 1116.4 / 121.8 ms, both exit code 0.
Working sets stayed approximately 74-77 MB during these short runs. See
`evidence/gui-smoke-repeat-1.json`, `evidence/gui-smoke-repeat-2.json`, and the
final no-game screenshot `evidence/gui-window.png`. These are short debug runs,
not release performance or sustained focus-cycle memory acceptance.

One intervening smoke run exceeded the 15-second process-wait limit. Temporary
diagnostics subsequently showed application, monitor and settings cleanup all
completed; the diagnostics were removed. A separate run exposed null ExitCode
from PowerShell Start-Process. The harness now explicitly retains a managed
process handle, records real game presence and writes failure JSON instead of
leaving stale passing metrics. Two repeats pass, but the isolated timeout's root
cause is not conclusively established; extended shutdown acceptance remains open.

## Hosted CI and remaining acceptance

Source revision 2c2bdfa passed hosted Windows and Ubuntu CI on Rust 1.95:
https://github.com/floris-xlx/DASRU/actions/runs/42191220518
The Windows job executed Raw Input unregister/repeated-start-stop tests, the new
overflow test, atomic settings tests and the release GUI build. These are backend
resource tests, not proof of physical GUI/TDA shortcut acceptance. The subsequent
smoke-harness/evidence-only revision must also be checked after pushing.

Merge remains blocked pending physical GUI hotkey dispatch/focus/hold/release,
listener stop/window-close Raw Input cleanup against TDA, game exit/restart,
unknown executable/access-failure live cases, native
125-200% DPI and monitor moves, release idle/memory-cycle performance, final
redistribution notices/project-license decision and PR review. The GUI exports to
a user-entered path; a native browse dialog is not implemented. Raw Input events now use a bounded 256-event queue; overload invalidates queued focus generations and terminates dispatch. OS I/O and an in-flight support export may delay close, although
executable hashing now cooperatively cancels between chunks.

No FPS limiting, input injection, macros, automation, OSD writes or memory writes
were added. This document records evidence and open gates, not a completion claim.

Additional regression: a pre-existing temporary settings file is preserved on creation collision (RED missing publish helper; GREEN ownership-aware publication). The Windows overflow test first failed compilation with missing overload state/error code; it compiles locally and passed hosted Windows execution. Windows orchestration was moved into windows/session.rs to keep the backend module below 1,000 lines.
