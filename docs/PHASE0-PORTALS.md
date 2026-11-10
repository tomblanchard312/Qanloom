# Phase 0 file picker reliability

Status 2026-11-08: initial E02 hardening; broader portal acceptance remains open. No dependency or package pins change.

Ninimma is optional. A version such as `ninimma 0.1.9` alone does not authorize launching `--file-dialog`: some releases only expose a file-manager window. A compatible implementation must include the exact additional line `file-dialog-protocol 1` in `--version` output. That line promises the existing version-1 JSON request and bounded JSON array response protocol, including empty-array cancellation. Do not advertise it until the actual picker works. Existing non-advertising installations immediately retain the native/built-in fallback.

Native cancellation is final, including an immediate Escape or empty response. The former timing threshold could reopen an unparented dialog or the built-in browser after a user cancelled. It has been removed. Missing portal/zenity preflight still falls back, and File → Open → Browse in WordCraft remains available if a backend cannot present a dialog. `WORDCRAFT_FILE_DIALOG=builtin` remains an explicit supported override.

The current rfd API collapses some backend errors into cancellation and can wait indefinitely for a lost response. This first patch deliberately makes no claim to solve portal restart, hung backend, explicit response-code distinction or cross-platform parenting. E02 must remain open until a typed portal path or upstream API supports and passes those cases. No timeout guessed from a user's response speed is acceptable.

Tests exercise markerless versions, protocol versions, a real helper process that must never receive a picker invocation without the advertisement, advertised cancellation, invalid selections, output bounds and immediate native cancellation. Complete native Open, Save, overwrite, cancellation, restart, Unicode and symlink workflows with source hashes and document round-trips before release. Keep Linux X11 development evidence separate from Plasma Wayland/FreeBSD and sandbox acceptance. This patch does not change save formats, document buffers, window themes, authentication or assistant identity.
