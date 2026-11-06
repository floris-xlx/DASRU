# Desktop UI TDD evidence

## Hardening outcome (2026-11-06; code head 98cf29c)

PR #10 remains a draft and is not ready for review/merge. No new branch or game
feature was introduced. Fetched main is still
`4e637752b9241cff7805e6ee9baf72c649557e10`, an ancestor of this PR branch.

| Gate | Current result |
|---|---|
| A Shutdown | FAIL - ownership/error gaps fixed and twelve no-game release closes pass, but historical >15s timeout remains unexplained; active-game close cases pending |
| B Real TDA | PENDING - operator explicitly unavailable; no physical shortcut/focus/restart acceptance observed |
| C Display | PENDING - real 720px native 100% capture passes on one 5120×1440 monitor; 125–200%, practical lower resolutions and monitor transitions untested |
| D Performance | PENDING - sustained no-game release sample passes; game/listener modes and repeated physical transitions untested |
| E Reliability | PENDING - automated regression/review checks pass; critical lifecycle/live gates A/B still open |
| F Distribution | BLOCKED - technical dependency/font/runtime review recorded; existing project-license/provenance clearance remains unresolved |

### Sustained release evidence

`evidence/gui-smoke-20261106T135136703Z/` records 900 raw samples over
910.075 seconds on clean code head 98cf29c, binary SHA-256
`4F8B21AA7AEB9410C0E7A1200B1AC43099DDA17387C20A58E224B36E5C0AE17E`.
No TDA and no listener were running; no game input was sent. Startup: 470.65ms;
process CPU: 19.859s, 2.182% of one logical core; working set: 67.70MB initially,
75.39MB finally, 76.50MB peak; maximum sampled WM_NULL response: 6.00ms; normal
close: 91.18ms; exit code zero with every shutdown phase present. MB here are
decimal bytes/1,000,000. This is a no-game observation, not proof of active input
registration cleanup or leak freedom across game/focus cycles.

Three-minute working-set window means: 70.77, 74.26, 75.57, 75.54, 75.44MB.
After warmup the last nine minutes show no sustained rise. The derived
`memory-windows.json` was corrected for a PowerShell nested-array grouping error;
the original 900 raw samples were not modified. Native conditions: Windows 10 Pro
19045, i9-14900K, 32 logical CPUs, single 5120×1440 display at 96 DPI.

The installed Steam manifest reports build 25755602. One normal Steam launch was
requested after the idle run; subsequent checks observed no TDA process, Steam's
Running=0 and Updating=0. The operator then confirmed physical acceptance was
unavailable. No repeated launches or synthetic key input were used. Both the GUI
and game were absent at the final local process check. Current physical accepted
counts, focus transitions, game restart and active-listener cleanup are untested.
Follow [the exact manual procedure](pr10-native-acceptance.md) when available.

### Regression, privacy and dependency review

Seventeen Rust tests were added in this phase (251 passing WSL tests versus the
234 baseline). Eleven demonstrated failing cases are recorded below, including
the compile-time missing retained-report API; six additional safeguards already
passed when introduced and are explicitly regression evidence. The release smoke
profile assertion also failed before its fix. Test commands used WSL2 throughout.

Rust 1.95 fmt, WSL/native warnings-as-errors Clippy, WSL all-target/all-feature
tests, native release GUI and CLI builds pass. Actual native CLI checks accepted
two configured observe bindings, rejected an unknown action with exit 2 and
HOTKEY_CONFIG_UNKNOWN_ACTION, and reported NO_PROCESS from probe. Existing PR #9
configuration, action IDs, generation fencing and CLI output order remain intact.

Reviewed the complete source diff against main, including GUI/controller/runtime,
Windows session extraction, core cancellation changes and MSRV Clippy edits.
Process/build projections remain fail-closed; fresh native attachment, pinned
identity and foreground checks remain authoritative. Cache metadata is not
authorization and expires at 30s. Queue bounds/overflow fencing, repeat suppression,
single observation counting and post-stop rejection remain covered. No GUI call
to game-memory writers, input injection or a CLI subprocess was introduced.
Preferences retain schema/validation, corrupt-file preservation, conflict checks
and atomic changed-value publication. Support export retains the core redaction
and create-new/no-overwrite implementation. No additional defect was demonstrated
in this review; unresolved acceptance/lifecycle limitations remain explicit above.

`evidence/pr10-distribution-review-20261106T1403Z.json` records locked Windows
metadata: all 129 registry packages occur in the existing 130-entry inventory;
none lack license metadata and the highest declared dependency MSRV is 1.95.
All five bundled font notice texts match epaint_default_fonts 0.37.0 verbatim.
No dependency, font, artwork or lockfile was added by hardening. PE imports identify
x64 MSVC/Universal CRT and OpenGL requirements. SPDX/font checks do not resolve
the existing README project-license/provenance caveat or constitute final
redistribution clearance; no project license was selected on the owner's behalf.

Code head 98cf29c passed [hosted CI run 42228762994](https://github.com/floris-xlx/DASRU/actions/runs/42228762994):
Windows 271 tests pass (one intentionally ignored child-process fixture), Ubuntu
251 pass, both fmt/Clippy pass, and Windows release GUI build passes. The evidence
commit after this code head has no runtime changes; its own final-head hosted
checks are verified after pushing and linked in the PR description.

Hardening commits: baseline 2f625b0; startup owner 1136d30; terminal owner 85b67f1;
queued work/retained errors 20a6938; trace/unchanged preferences 98cf29c. No merge
or force push was performed. Remaining gates require actual game/operator/display
acceptance and final licensing/provenance review; compilation is not substituted
for those results.

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

### A2/A3 - startup-abort thread ownership

Requirement: shutdown never leaves a listener owner detached.

Observed defect: `abort_startup` discarded its live `JoinHandle` when the
completion notification timed out or disconnected. Extracted that existing
notification/join sequence unchanged into the platform-independent private
`hotkeys::worker` module so WSL can exercise the actual production path.

Failing tests: `startup_abort_joins_after_completion_disconnect` and
`startup_abort_joins_after_completion_timeout`. Both failed on Rust 1.95 with
`cleanup detached a live resource owner`. Channel-controlled workers are released
and joined before assertions even in the failing run; no injected keyboard input
or game is involved. The 100ms receive deadline detects an early return, rather
than claiming a wall-clock shutdown bound.

Minimal implementation: join the owned thread regardless of completion-channel
outcome. Startup still returns its original startup error. GREEN: both tests
passed. Refactor: rustfmt only. Native Windows Rust 1.95 all-target/all-feature
check passed. Raw RED/GREEN outputs are retained under `docs/evidence/`.
Regression: 236 Rust 1.95 WSL all-target/all-feature tests passed; native Windows
warnings-as-errors Clippy passed. Commit: `Fix startup cleanup retaining the Raw
Input worker owner` (this section is committed with the fix).

This fixes a demonstrated ownership error, not the unexplained historical
normal-close timeout. Joining can wait on an uninterruptible OS call. The separate
normal-shutdown fallback and final cleanup-error reporting remain under review;
Gate A remains FAIL.

### A2/A3 - terminal Raw Input ownership on shutdown errors

Requirement: cleanup notification failure must not detach a resource owner or
erase the failure. The same early-return pattern existed in normal shutdown;
after repeated errors `Drop` could discard the remaining live handle.

Failing tests: `shutdown_joins_owner_after_notification_failure` and
`shutdown_joins_owner_after_notification_timeout`, both failed with `shutdown
returned with a live owner`. GREEN: both pass after always joining before
returning the original notification outcome. Backend cleanup failure and owner
panic tests already passed before the fix and remain regression checks, not
claimed RED cycles. Six ownership tests now pass. The backend retains the abort
flag and checks it after waking, including when posting STOP fails. A blocked
Windows call still has no safe hard interruption guarantee.

Refactor: the existing completion/join sequence lives in a private portable
helper used by Windows; no input or dispatch logic changes. Windows Rust 1.95
all-target/all-feature check and warnings-as-errors Clippy pass. WSL Rust 1.95
regression: 240 tests pass. Raw outputs: `evidence/pr10-shutdown-join-*-*.log`.
Commit: `Join Raw Input worker before returning shutdown failures`.
Gate A remains FAIL pending final error reporting and instrumented native closes.

### A2/A3 - cancel queued work, finish owned I/O, preserve cleanup failure

Requirement: shutdown cancels queued work, does not start another probe after a
cancelled in-flight export, flushes final preferences, and reports cleanup errors
beyond UI destruction.

Observed defects and RED:

- `shutdown_cancels_queued_export_and_rejects_new_work` failed: Pending instead
  of Failed(SUPPORT_EXPORT_CANCELLED) after the monitor joined.
- `shutdown_during_export_joins_without_starting_another_probe` failed: probe
  count rose from 1 to 2 after cancellation during the channel-controlled export.
- `shutdown_failure_survives_app_destruction_and_repeated_close` initially failed
  compilation because shutdown returned unit and no retained report existed.

Minimal implementations: cancel the queued destination under its mutex; check
cancellation after in-flight export; check new exports against cancellation under
the same queue lock. The default service export remains `support::export_json`;
only its service boundary is injectable for deterministic tests. Preserve the
first shutdown error while still finishing both owners. Retain the terminal
result in a shared report read by the native entrypoint after eframe returns;
cleanup failure now returns a nonzero process status. Repeated close returns the
same result. No state or I/O lock is held across a join.

GREEN: all four controller tests and the retained-report test pass. Settings
queue/final-flush and write/panic failure tests pass as additional regression
coverage; they did not expose a defect and are not claimed RED cycles. The
settings save closure is extracted without changing storage semantics, atomic
publication or corrupt-file preservation. Rustfmt applied; native Rust 1.95
all-target/all-feature check passed. Raw cycle outputs are under
`evidence/pr10-*-20261106T1340Z-*.log`.

In-flight support export and settings filesystem calls remain cooperative,
uninterruptible waits owned until completion. No hard deadline or forced thread
termination is introduced. This still does not establish the historical timeout's
cause; Gate A remains FAIL until instrumented acceptance and remaining review.
Regression checks: 245 WSL Rust 1.95 all-target/all-feature tests passed; native
Windows warnings-as-errors Clippy passed. Commit: `Cancel queued GUI work and
retain terminal cleanup failures`.

### A4 - opt-in timing and repeated native release closes

New `--shutdown-trace <new-file>` uses the existing tracing dependency, an
exact-target filter, static phase codes and monotonic timestamps. RED:
`trace_contains_phases_without_unrelated_sensitive_events` found an unrelated
fixture event; `app_shutdown_has_one_terminal_phase_on_repeated_close` found no
terminal phase. A further RED test found three controller cancellation markers
across explicit close/repeat/Drop instead of one. GREEN: all five trace tests
pass, including invalid arguments and destination no-overwrite safeguards.
Refactor: diagnostic writes stay outside shared controller locks. EFRAME_RETURNED
separates application cleanup from subsequent renderer/window teardown.

Smoke harness RED: a copied harness in an isolated scratch directory ran the
release executable but labelled it debug. Historical evidence was not overwritten
for this test. The harness now defaults to release, records binary hash/head/dirty
state, creates a new timestamped directory per run, checks exit and shutdown
phases, supports bounded extended samples, and leaves failed processes available
for diagnosis instead of counting forced termination as cleanup. GREEN: repeated
runs assert release metadata and preserve hashes of the historical evidence files.

Native Windows 10 Pro 19045, i9-14900K (24 cores/32 logical CPUs), one 5120×1440
monitor at 96 DPI. Eleven instrumented release runs without TDA/listener passed:
one ten-second sample, then five one-second transition samples before and five
after the unchanged-preferences fix below. Per-run JSON, screenshot, samples and
trace are retained in `evidence/gui-smoke-20261106T*/`. The first release sample
showed 488ms startup, 2.31% of one core, 67.5–69.9MB working set and 6.1ms maximum
WM_NULL response. These short samples are not sustained acceptance.

Three pre-optimization closes took 1.68–2.31s. One trace placed ~2.22s in settings
flush; two had delay between the first phase and cancellation logging. Synchronous
diagnostic writes/scheduling can add overhead; these traces do not prove the
cause of the historical >15s close. No new >15s timeout was observed.

### A3 follow-up - avoid unchanged-settings durable publication

Requirement: avoid unnecessary I/O after measured shutdown cost while preserving
validation and conflict protection. RED:
`unchanged_preferences_avoid_republication_without_skipping_conflict_checks`
failed because a second identical save replaced the file and changed its fixed
modification time. Minimal GREEN: compare serialized bytes only after validation
and reading/checking the original destination. Identical content returns without
republishing; externally changed content still fails and is preserved. All seven
preferences tests pass. Refactor: rustfmt. Five subsequent native normal closes
took 89–106ms, startups 491–512ms, WM_NULL responses <=6.94ms; all exit codes zero
and all required phases recorded. No listener registration was exercised.

Final checks for this slice: 251 WSL Rust 1.95 tests pass; native Rust 1.95 fmt,
warnings-as-errors Clippy and locked release build pass. Raw outputs are under
`evidence/pr10-*-20261106T1350Z-*.log`. No dependencies/lockfile changed.

Gate A remains FAIL: the historical >15s timeout is unexplained and active-game
close cases are unobserved. The policy retains resource owners and reports errors;
it makes no hard deadline for stalled OS I/O. B remains PENDING (TDA absent), C
partially observed at native 100% only. The exact outstanding physical/DPI/perf
procedure is in [pr10-native-acceptance.md](pr10-native-acceptance.md).

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
