# PR #8 TDD and acceptance evidence

## Red to green record

- **Binding parser and CLI:** `tests/hotkey_bindings.rs` was added before the parser. The initial CLI run failed the valid-binding, canonicalization, duplicate, reserved, and malformed-key cases because the existing CLI returned generic usage instead of binding-specific errors. After parsing and duplicate validation were added, all 10 binding tests passed.
- **Lifecycle and foreground policy:** `tests/hotkey_lifecycle.rs` was added before the manager. Its initial test build failed because the manager and state types did not exist. After adding the seam, `verified_process_outside_foreground_starts_inactive` failed with `InvalidTransition`; `conflict_is_not_retried_until_focus_changes` failed because registration retried without a focus transition. Both failures led to the state-machine fixes. All 26 lifecycle cases now pass.
- **Capability reporting:** the first evaluator tests found `GlobalHotkeys` reported as unimplemented. The later test-first regression run `cargo test --locked --test hotkey_capability active_registration` failed both new cases: registration alone returned `Supported` instead of `Unknown` without process evidence and `Unsupported` for an unknown build. `failed_process_evidence_blocks_hotkey_support` also failed because invalid process evidence returned `Supported`. The evaluator now requires verified process identity, exact-build evidence, and active registration. All 8 capability cases pass.
- **Keyboard tracking:** `tests/hotkey_input_events.rs` was added before the tracker. Its first build failed because `hotkeys::keyboard` did not exist. The four tests now cover make/break, left/right modifier state, repeat suppression, and focus-loss reset.
- **Raw Input lifecycle:** two manager tests were added before `handle_keyboard_input`; their first build failed because the method did not exist. They now verify one dispatch per chord make and that focus reactivation cannot reuse stale modifier state. A regression test also checks that a Raw Input registration failure preserves its structured error code.
- **Focus ordering replay:** the first deterministic focus-cycle replay observed the bug: a Ctrl+Alt+F9 sequence queued before focus loss produced one accepted event after refocus. The generation-aware keyboard replay test now produces one chord candidate in the current generation, advances through focus loss/refocus, then replays the old-generation chord and observes zero candidates. The lifecycle suite also verifies keyboard rejection after focus loss, immediate rejection after process-identity change, and balanced registration/unregistration over eight focus cycles. Diagnostic counter tests verify independent stage counts and saturation at `u64::MAX`.
- **Windows input backend:** the earlier `RegisterHotKey` tests were replaced after the live investigation showed `WM_HOTKEY` was not delivered in TDA. The five current Windows tests cover Raw Input being absent while inactive, registration on activation, removal on deactivation and shutdown, repeated start/stop, coexistence with another owner’s global hotkey registration, and message-loop termination. Focus transitions also discard queued input before the manager changes state, preventing an old chord from carrying across a focus cycle. They use no synthetic keyboard input. The final assertions passed in hosted Windows CI run `42172509846`.

## Local verification

Commands were run from WSL2 as required for Cargo tests:

- `cargo test --locked` - passed from WSL2 after the focus-ordering change: 94 library tests and all integration suites, including 10 binding, 8 capability, 4 keyboard, and 26 lifecycle tests.
- `cargo fmt --all -- --check` - passed.
- `cargo clippy --locked --all-targets -- -D warnings` - passed on Linux.
- `cargo clippy --locked --all-targets --target x86_64-pc-windows-gnu -- -D warnings` - passed from WSL2, compiling the Windows backend and Windows-only tests.
- Native Windows `cargo clippy --locked --all-targets -- -D warnings` - passed before the final focus-lifecycle adjustment; final Windows code also passes WSL2 Windows-target Clippy.
- An earlier local native Windows run passed the 5 Raw Input lifecycle tests before the final focus-lifecycle assertions were added. The final updated Windows tests passed in hosted Windows CI run `42172509846`. A full local native Windows test run was not repeated while the live TDA process was running because `hotkeys_command_does_not_start_without_a_verified_process` can start a listener when a verified game is present; the earlier attempt did start such a listener, which was stopped.

The final Windows backend tests assert registration occurs only on activation and is removed on deactivation and shutdown, plus repeated start/stop, coexistence with another owner’s global hotkey, and message-loop termination. Hosted Windows CI run `42172509846` executed those tests successfully.

Hosted run `42162658883` first exposed that two valid-binding tests wrongly expected the Linux-only `HOTKEY_UNSUPPORTED_PLATFORM` result on Windows. Those assertions now exercise the pure parser. The follow-up run `42163079270` passed on both Windows and Ubuntu before the Raw Input change. Current Raw Input code passed Windows and Ubuntu in PR run `42172509846` and push run `42172510313`, including formatting, Clippy, and `cargo test --locked`.

## Known limits and acceptance still required

- Raw Input does not reserve the configured chord system-wide. DASRU unregisters the keyboard device on focus loss, drains queued messages at focus boundaries, tags forwarded input with a focus generation, discards stale generations, and rechecks foreground identity at dispatch.
- The listener requires exactly one accessible TDA process and a SHA-256 match to a registered executable build. Unknown builds fail closed.
- No synthetic key input, input injection, macros, FPS-cap writes, game-memory writes, or game actions were added.
- Linux tests and Windows integration tests do not establish physical keyboard delivery or game restart/exit behavior. Focus deactivation/reactivation and diagnostic resource cleanup were observed live. The production listener’s physical acceptance was not demonstrated. Live conflict handling and game restart/exit remain untested.

The investigation used a temporary native Windows harness to compare `WM_HOTKEY`, `WM_INPUT`, and `WH_KEYBOARD_LL` while recording only modifier/F9 virtual-key codes, foreground PID, verified identity status, and source counts. The harness was removed after the investigation. Earlier runs received `WM_HOTKEY` on the desktop but not with TDA foregrounded. Raw Input and the low-level hook both reported two `WOULD_ACCEPT` chord makes during an earlier TDA-focused run, demonstrating that both sources can observe the chord in that context; this did not prove the production listener’s dispatch path. The low-level hook is diagnostic only and is not used by the production backend.

In the latest diagnostic attempt, the user’s physical F9 press was seen by `WH_KEYBOARD_LL` as `Ctrl+Alt+F9` with the verified TDA process foregrounded and marked `WOULD_ACCEPT`. The corresponding `WM_INPUT` F9 make was processed after foreground had changed to another process; it had no tracked modifiers and was rejected as `CHORD_MISMATCH`. On shutdown the harness reported `raw_removed=true`, `hotkey_unregistered=true`, `low_level_unhooked=true`, `foreground_unhooked=true`, and `all_released=true`. This is consistent with the required fail-closed focus check, but it leaves Raw Input acceptance in the production path unresolved.

The actual DASRU listener was then run against the verified TDA process. It reached `Active` when TDA was foregrounded and returned to `Inactive` on focus loss, but it emitted no `HOTKEY_ACCEPTED` line for the user’s repeated attempts. The listener was stopped. Thus the manual success criterion of exactly one accepted event while TDA is foregrounded and zero accepted events outside TDA is not met. The Raw Input backend remains the selected implementation because it avoids system-wide chord reservation, but Gate D remains failed pending a reliable TDA-focused production event.

## Gate status

- Gate A - TDD evidence: new pure keyboard and lifecycle behavior tests were demonstrated failing before implementation, then passing.
- Gate B - Local validation: WSL2 tests, format, Linux Clippy, and Windows-target Clippy passed; the final Windows API lifecycle tests passed in hosted Windows CI.
- Gate C - Hosted Windows and Ubuntu CI: passed on PR run `42172509846` and push run `42172510313` for commit `e161335`.
- Gate D - Live Windows + TDA acceptance: failed/incomplete. Diagnostic Raw Input and the actual listener did not produce a verified production acceptance for the latest physical attempts.
- Gate E - PR review: pending; no merge has been performed.
