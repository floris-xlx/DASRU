# PR #9 TDD and acceptance evidence

## Red to green record

- **Versioned configuration:** `tests/hotkey_config.rs` first failed to compile because `hotkeys::config::HotkeyConfig` did not exist. After adding the parser, all 11 tests passed for schema version, unknown fields/actions, size and binding limits, duplicates, disabled entries, reserved shortcuts, and empty active configuration.
- **Typed action resolution:** the initial `tests/hotkey_dispatch.rs` run failed because `hotkeys::dispatch` did not exist. After adding `ConfiguredBinding`, its five resolution tests passed for primary/secondary IDs, shortcut normalization, unknown shortcuts, and disabled bindings.
- **Read-only dispatch:** dispatch tests first failed to compile because `ActionDispatcher`, `ActionEvent`, and `ActionEventSink` were missing. The initial green run exposed an incorrect test expectation for a key after shutdown; the assertion was corrected to require no event after the manager clears modifier state. The suite now verifies a single event per physical key make despite repeats, secondary ID resolution, disabled/unconfigured binding rejection, foreground and identity rejection, stale generations, modifier reset, and shutdown.
- **CLI:** before CLI integration, `cargo test --locked --test hotkey_cli` failed four cases because `validate --config` and `listen --config` returned generic usage errors instead of validating configuration or reporting stable config errors. The mixed `--config`/`--bind` rejection already passed. After integration, all five CLI tests pass, including path redaction and validation before the backend starts. Existing legacy `listen --bind` CLI tests remain passing.
- **Windows backend reuse:** the PR8 Raw Input backend and manager remain in place. Added a Windows regression asserting two enabled bindings share activation and that Raw Input remains registered until both bindings deactivate. Existing tests cover partial registration rollback, focus loss, generation rejection, legacy single-binding behavior, and shutdown cleanup.

## Configuration and dispatch contract

Example `hotkeys.toml`:

```toml
schema_version = 1

[[bindings]]
id = "primary"
shortcut = "CTRL+ALT+F9"
action = "observe"
enabled = true

[[bindings]]
id = "secondary"
shortcut = "CTRL+ALT+F10"
action = "observe"
enabled = true
```

`dasru hotkeys validate --config hotkeys.toml` validates without attaching to a game. `dasru hotkeys listen --config hotkeys.toml` loads the same versioned file at startup. The file is limited to 64 KiB and 16 enabled bindings. IDs are unique across all entries; enabled shortcuts must be unique after normalization. Unknown fields, schema versions, actions, reserved shortcuts, and empty active configurations fail before the Windows input backend starts. Disabled entries are not dispatched. `observe` only writes a diagnostic line.

The legacy `dasru hotkeys listen --bind CTRL+ALT+F9` path remains supported. `--bind` and `--config` cannot be combined.

## Local verification

Cargo commands were run in WSL2:

- `cargo fmt --all -- --check` - passed.
- `cargo test --locked` - passed: 94 library tests and all integration suites, including 11 config, 13 dispatch, 5 CLI, and existing lifecycle, capability, and contract regressions.
- `cargo clippy --locked --all-targets -- -D warnings` - passed on Linux.
- `cargo clippy --locked --all-targets --target x86_64-pc-windows-gnu -- -D warnings` - passed, compiling Windows backend code and Windows-only tests.
- `git diff --check` - passed.

Hosted Windows and Ubuntu PR CI passed in run `42184375151`, including formatting, warnings-as-errors Clippy, and `cargo test --locked` on both runners.

## Scope and remaining acceptance

- The action sink only reports `observe`; no macros, simulated input, FPS-cap changes, GUI, or game-memory writes were added.
- Dispatch requires the existing pinned-process identity and foreground checks, then rechecks the input focus generation before emitting.
- **Live TDA acceptance (configured bindings):** the native Windows build succeeded and validation reported `bindings=2`. In a controlled run, TDA was focused for three separate Left Ctrl+Alt+F9 taps and three separate Left Ctrl+Alt+F10 taps. The listener emitted exactly three `HOTKEY_ACCEPTED` and three matching `HOTKEY_ACTION_DISPATCHED id=primary action=observe` lines for F9, and the same counts for F10 with `id=secondary`. During the deliberate unfocused interval it reported `Inactive` and emitted no accepted or action-dispatch lines. Refocusing TDA returned it to `Active`.
- **Configured listener cleanup:** Ctrl+C ended with `HOTKEY_STATE state=Disabled`. Its bounded summary reported `raw_observed=68`, `forwarded=68`, `forward_failed=0`, `read_failed=0`, `queue_discarded=0`, `stale_dropped=0`, `processed=68`, `chord_candidates=6`, `accepted=6`, `rejected=0`, `identity_rejected=0`, `focus_changes=39`, `activations=4`, and `deactivations=4`.
- **Legacy `--bind` check:** `listen --bind CTRL+ALT+F9` started, activated once, deactivated on focus loss, and shut down as `Disabled` with zero accepted events. This verifies legacy CLI and registration lifecycle; a physical event in legacy mode was not observed.
- **Gate D:** configured physical delivery, focus filtering, exact counts, and active-session shutdown passed. The separate legacy physical-event check and Gate E review remain open, so PR #9 stays draft and unmerged. CI does not replace live evidence.
