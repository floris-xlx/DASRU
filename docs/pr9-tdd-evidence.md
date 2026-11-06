# Desktop UI TDD evidence

Base: 378063005c3669426b87e6d5d1343b92f8ce02eb.
Branch: feat/desktop-ui-foundation. Draft PR #10 (#9 already existed).

## Observed Red ? Green cycles

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

Final debug-build sample at 96 DPI (100%): startup to visible window 437.9 ms;
10-second sample consumed 3125 ms process CPU (0.3125 of one logical CPU), working
set 86,937,600 ? 88,244,224 bytes; maximum WM_NULL response 9.84 ms; normal shutdown
140.2 ms, exit code 0. Sampling includes initial monitoring and rendering, so it
is not a release idle-CPU benchmark or a sustained-memory stability test.
The screenshot shows actual Verified process status and registered build
`steam-25755602`. See `evidence/gui-window.png` and `evidence/gui-smoke.json`.

## Hosted CI and remaining acceptance

Bootstrap commit b5ad467 passed hosted Windows and Ubuntu checks, including the
Windows release build. Those results do not validate the subsequent implementation;
latest-commit hosted results must be checked after pushing.

Merge remains blocked pending physical GUI hotkey dispatch/focus/hold/release,
listener stop/window-close Raw Input cleanup against TDA, game exit/restart,
unknown executable/access-failure live cases, no-game native launch, native
125-200% DPI and monitor moves, release idle/memory-cycle performance, final
redistribution notices/project-license decision and PR review. The GUI exports to
a user-entered path; a native browse dialog is not implemented. Raw Input events now use a bounded 256-event queue; overload invalidates queued focus generations and terminates dispatch. OS I/O and an in-flight support export may delay close, although
executable hashing now cooperatively cancels between chunks.

No FPS limiting, input injection, macros, automation, OSD writes or memory writes
were added. This document records evidence and open gates, not a completion claim.

Additional regression: a pre-existing temporary settings file is preserved on creation collision (RED missing publish helper; GREEN ownership-aware publication). The Windows overflow test first failed compilation with missing overload state/error code; it now compiles locally and awaits hosted execution. Windows orchestration was moved into windows/session.rs to keep the backend module below 1,000 lines.
