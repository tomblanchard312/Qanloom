# Qanloom TavruOS interface

Updated 2026-11-09. Audience: people writing, reviewing and exchanging documents.

## Design contract

Follow the [approved purple workspace](https://github.com/tomblanchard312/TavruOS/blob/main/design-system/PURPLE_WORKSPACE.md)
and [interface requirements](https://github.com/tomblanchard312/TavruOS/blob/main/design-system/INTERFACE_RULES.md).
Consume shared semantic colors, Inter typography and Phosphor icons from
[tavru-style](https://github.com/tomblanchard312/tavru-style), pinned in Cargo.lock.
The dependency's `eridu-style` package name is retained for compatibility.

The primary task is writing. Keep the ribbon command groups and precise document
pages, rather than imposing a media gallery. File navigation uses a responsive
180–260px sidebar, 48px scrollable primary rows, rounded selection and explicit
keyboard focus. Workspace gutters are 24px. Standard controls use 10px corners,
dialogs and assistant surfaces 16px. Dense ribbon controls retain compact hit
areas pending a separately verified ribbon density redesign.

Follow the system theme and saved appearance preference. When the system gives
no preference, use dark. Both themes use shared purple actions, lavender links
and tonal surfaces. Document fonts, colors, page geometry and file formats remain
independent of interface styling. Preserve command IDs, configuration and data.

## Workflows and states

- First run: File Home offers New, Open and recent documents.
- Populated: edit pages, format with the ribbon, save/reopen, review changes.
- Assistant: existing local assistant integration remains optional and local. The toolbar
  shows **Ask** with Phosphor ChatText; the pane uses the canonical Oqelith mark
  without a name or wordmark. Preserve its
  checking, unavailable, busy, error and stale-suggestion states, labelled composer,
  Stop/Retry controls and document revision guards. Do not rename its product or
  replace its mark as part of this visual migration.
- Recovery: retain unsaved-work prompts and recovery pages. Escape and keyboard
  focus behavior stay intact. No new privileged or terminal workflow is introduced.

## Acceptance and remaining work

Linux is the primary native target, FreeBSD secondary. Existing macOS, Windows
and WASM targets remain part of the architecture. This change uses offscreen
screenshots and UI regressions, not a claim of native platform acceptance.

Release acceptance still requires native Orca document reading, keyboard-only
workflows, 100/150/200% scaling, long translations, reduced motion, narrow windows,
and both themes. Qanloom maintainers own these checks and remaining compact ribbon
targets. Palette and geometry adoption alone do not establish full accessibility
or MVP readiness. See the final validation report for checks actually completed.

### Validation, 2026-11-09

- Final UI regression binary with loopback access: 1,046 passed, five ignored.
  The untranslated-interface audit was excluded from this run.
- UI package Clippy with all targets and denied warnings passed.
- Asset attribution and dependency layering passed. WASM check passed with
  existing unused-code warnings.
- Offscreen light/dark editor and File Home screenshots inspected, plus an
  800px-wide assistant and Home. Narrow templates now wrap, pinned by a test.
- Full workspace CI stopped at two pre-existing needless-borrow Clippy errors
  in the uncommitted `apps/wordcraft/src/file_dialogs/ninimma.rs` work. That
  independent work was preserved. No commit, PR or release readiness claimed.
