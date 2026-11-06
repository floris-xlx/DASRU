# Desktop architecture (bootstrap)

`dasru-gui` is a feature-gated native eframe application using Glow on Windows.
The CLI continues using the existing library. Linux default builds have no GUI
dependencies; enabling desktop-ui permits headless egui rendering tests without
compiling eframe. The rendering module currently performs no core operations.

The controller, service boundary, reusable hotkey runtime and preferences are
outstanding. Do not treat the navigation shell as a functional implementation.
