# PR #8 TDD and acceptance evidence

## Red to green record

- **Binding parser and CLI:** `tests/hotkey_bindings.rs` was added before the parser. The initial CLI run failed the valid-binding, canonicalization, duplicate, reserved, and malformed-key cases because the existing CLI returned generic usage instead of binding-specific errors. After parsing and duplicate validation were added, all 10 binding tests passed.
- **Lifecycle and foreground policy:** `tests/hotkey_lifecycle.rs` was added before the manager. Its initial test build failed because the manager and state types did not exist. After adding the seam, `verified_process_outside_foreground_starts_inactive` failed with `InvalidTransition`; `conflict_is_not_retried_until_focus_changes` failed because registration retried without a focus transition. Both failures led to the state-machine fixes. All 21 lifecycle cases now pass.
- **Capability reporting:** the first evaluator tests found `GlobalHotkeys` reported as unimplemented. The later test-first regression run `cargo test --locked --test hotkey_capability active_registration` failed both new cases: registration alone returned `Supported` instead of `Unknown` without process evidence and `Unsupported` for an unknown build. `failed_process_evidence_blocks_hotkey_support` also failed because invalid process evidence returned `Supported`. The evaluator now requires verified process identity, exact-build evidence, and active registration. All 8 capability cases pass.
- **Win32 backend:** the six Windows integration tests were written before `WindowsBackend`; the initial Windows-target test check failed because that backend type did not exist. Tests cover `registers_and_unregisters_valid_hotkey`, `reports_existing_hotkey_conflict`, `releases_binding_after_shutdown`, `repeated_start_stop_releases_resources`, `partial_manager_registration_failure_rolls_back_prior_binding`, and `message_loop_terminates_on_shutdown`. The backend now owns registration, unregistration, the foreground event hook, and message loop on one thread. Windows-target compilation and Clippy pass; native Windows execution remains a hosted CI gate.

## Local verification

Commands were run from WSL2 as required for Cargo tests:

- `cargo test --locked` - passed: 91 library tests and all integration suites, including 10 binding, 8 capability, and 21 lifecycle tests.
- `cargo fmt --all -- --check` - passed.
- `cargo clippy --locked --all-targets -- -D warnings` - passed on Linux.
- `cargo clippy --locked --all-targets --target x86_64-pc-windows-gnu -- -D warnings` - passed, compiling the Windows backend and its Windows-only tests.
- `cargo build --locked` - passed with the native Windows MSVC toolchain.

The Windows-only registration tests cover successful registration and release, an existing shortcut conflict, repeated start/stop, partial-registration rollback, and message-loop termination. Cross-target compilation does not execute those tests; Windows CI must do so.

## Known limits and acceptance still required

- `RegisterHotKey` reserves the chord system-wide while active. Focus-loss deactivation and event-time revalidation reduce interference, but a focus transition can race with Windows event delivery.
- The listener requires exactly one accessible TDA process and a SHA-256 match to a registered executable build. Unknown builds fail closed.
- No synthetic key input, input injection, macros, FPS-cap writes, game-memory writes, or game actions were added.
- Physical shortcut delivery, filtering after switching windows, game restart/exit behavior, conflict behavior on the live desktop, and cleanup after listener exit require Windows + TDA acceptance. They are not established by Linux tests or cross-target compilation.

## Gate status

- Gate A - TDD evidence: documented above.
- Gate B - Local validation: WSL2 tests and format check, Linux/Windows-target Clippy, and native Windows build passed.
- Gate C - Hosted Windows and Ubuntu CI: pending.
- Gate D - Live Windows + TDA acceptance: pending physical-key confirmation.
- Gate E - PR review: pending; no merge has been performed.
