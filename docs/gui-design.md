# Desktop design

Native resizable window: default 1000 x 680, minimum 860 x 560 logical pixels.
Four sidebar destinations, shared dark industrial tokens in `src/gui/theme.rs`,
reserved red selection, no gradients, expanded tracking, custom SVG or borrowed
artwork. Light theme and text scale are implemented preferences.

Dynamic status and numeric values use disabled inputs. Controls share six-point
rounding. Saving/exporting controls use skeletons while work is pending. Known
unavailable starts and invalid settings are disabled with visible reasons.
Scrollable screen content keeps controls reachable at the minimum layout size.
Refresh uses an approximately one-second monitor interval and scheduled repaint,
rather than a continuous frame loop.

The default-font fallback is egui's bundled redistributable font set; no system
font is copied. See `gui-font-notices.txt` for font notices and
`gui-dependency-licenses.json` for the Windows dependency inventory. SPDX options
are permissive (select Apache where a GPL alternative exists); font obligations
include OFL-1.1 and Ubuntu-font-1.0. Several published GUI crates omit license
files from their crate archives. Final redistribution needs upstream notices and
a project-license decision; README currently explicitly selects no DASRU license.

Headless tests exercise all screens at 100%, 125%, 150% and 200% scaling.
This establishes renderability, not pixel-level Windows DPI acceptance. Native
100% game-running and no-game screenshots and smoke measurements are in `docs/evidence`. Physical shortcut,
high-DPI and monitor-move acceptance remain open.
