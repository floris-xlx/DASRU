# Desktop UI specification - compact utility revision

This specification supersedes the original sidebar/dashboard direction for PR #10.
The visual reference is DESRU's utility layout philosophy, using the user's revised
two-column layout. DASRU retains Rust, egui/eframe 0.37 and the existing core.

## Window and hierarchy

- Fixed client width: 720 logical pixels (within the requested 650-750 range).
- Default height: 700; vertically resizable from 480 to 1200 logical pixels.
- No sidebar, page navigation or separate primary-control screens.
- Red, centered DOOM TDA SPEEDRUN UTILITY title above paired gray panels.
- Row 1: KEYBINDS / GAME STATUS.
- Row 2: FPS LIMITER / OPTIONS.
- Row 3: RESOLUTION SCALING / GAME VERSION.
- Full-width TRAINER / METRICS section.
- Bottom SETTINGS, HELP and DIAGNOSTICS buttons always remain accessible.

Primary controls fit together at the default size. Short windows, increased text
scale, or expanded details use vertical scrolling; the bottom dialog buttons stay
fixed. Paired panels share top alignment and minimum row heights. Controls and
panel borders use a consistent small three-point corner radius, preserving the
workspace rule against square borders while retaining a rectangular utility feel.
No gradients, expanded letter spacing or custom SVG are introduced.

## Functional controls and unavailable features

KEYBINDS edits the single implemented observation shortcut (Ctrl+Alt+F9 by default).
OPTIONS starts and stops the real shared listener, retaining binding, process,
build, focus and cleanup safeguards. The binding cannot be edited while listening.
GAME STATUS and GAME VERSION show actual process/build/listener evidence and a
refresh action. Dynamic read-only values use disabled inputs.

More hotkeys / advanced expands observation counters and the last event. It states
that additional bindings are not implemented; no fake FPS-toggle bindings are
shown. FPS limiting, freescroll, resolution scaling and trainer operations are
explicitly unavailable, with operational controls omitted. No fabricated FPS or
scaling values are displayed. Metrics are explicitly not collected by this UI.

## Dialogs

Settings, Help and Diagnostics open separate, closable egui dialog windows over
the utility, not navigation pages. These are in-application windows, not new native
OS windows. Dialogs are approximately 600 logical pixels wide and scroll when
needed. Only one auxiliary dialog is open at once; closing it leaves the primary
utility intact. Dialog text honors the same theme and text-scale preferences.

Settings retains validated, atomic preferences. Diagnostics retains capability
evidence, structured failures, observation counters and privacy-safe support export.
Help explains verification, foreground-only observation, stopping and exporting.
No game action, macro, injection, FPS write or memory modification is introduced.

## Typography, provenance and validation

Use the existing redistributable fallback fonts with uppercase section titles and
reserved red title treatment. DESRU-specific fonts or artwork are not copied or
bundled without independently verified redistribution rights. The reference is
https://github.com/bowsr/DESRU and the supplied layout diagram; its C# implementation
is not reused. Existing dependency/font notices remain applicable.

Legacy wide-window preferences normalize to the fixed width without losing their
binding/theme or overwriting the settings file during load. Corrupt-file and
unknown-schema preservation remain unchanged.

Headless tests cover primary-panel presence, column alignment, visible footer,
unclipped status fields, dialog open/close, text scaling, and 100-200% rendering.
Current native screenshot and width/close smoke evidence: evidence/gui-window.png
and evidence/gui-smoke.json. Earlier sidebar screenshots and measurements retained
in the TDD history are historical and do not validate this revised presentation.
Physical shortcut, native high-DPI/monitor and sustained performance gates remain
open; the original isolated close-timeout investigation remains recorded.
