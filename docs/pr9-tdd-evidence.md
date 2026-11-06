# Desktop UI TDD evidence

Base: 378063005c3669426b87e6d5d1343b92f8ce02eb.

## WP-01 (partial)

RED: `cargo build --locked --features desktop-ui --bin dasru-gui` failed:
`the package dasru does not contain this feature: desktop-ui`.

RED: bootstrap tests failed with missing `dasru::gui` and `egui`.
The first test invocation accidentally used native Windows; subsequent tests use WSL2.

GREEN: native Windows debug build passed with egui/eframe pinned at 0.37.0,
Windows-only eframe and explicitly selected Glow. CLI source is untouched.
The navigation tests cover initialization without game access and selection persistence.

## Outstanding gates

This is an incomplete bootstrap, not the functional desktop delivery.
WP-02 through WP-08 remain unimplemented. WP-04 has only preliminary navigation.
No monitoring, listener controls, diagnostics export, preferences, or shutdown
controller is wired. The window explicitly identifies monitoring as disconnected.
No live window, game, physical shortcut, DPI, screenshot, performance, resource
cleanup, hosted CI or review acceptance is claimed.

## Dependency review

`cargo info eframe@0.37.0` confirms MSRV 1.95 and MIT OR Apache-2.0.
The lockfile was resolved under the declared MSRV. No external artwork or fonts
were added; eframe's default_fonts feature uses its bundled fallback fonts.
A complete transitive redistribution license review remains outstanding.

Additional verification: `cargo +1.95.0 check --locked --all-targets --all-features`
passed on Windows. Formatting and Windows all-feature Clippy passed. The MSRV
bump required seven Clippy-suggested collapsible-if syntax changes in existing
core code. Headless rendering initially failed on an unapplied texture delta;
the test now explicitly clears deltas, as required by egui 0.37.
CI now selects Rust 1.95 and checks all features on both platforms, with a Windows
release GUI build. Hosted results remain pending.
`cargo test --locked --all-targets --all-features` passed under WSL2 after the harness correction (Windows-only tests excluded by target).
