# Tavru workspace geometry — 2026-11-10

The app previously overrode the shared style with 8px window, 6px menu and 4px control corners. It now pins the same immutable Tavru provider used by the desktop milestone and reads its SURFACE/CONTROL roles (16/12px). Open controls receive the same radius. This changes chrome only; pages, ruler, document styles, ribbon density, command/IO/undo behavior, automatic light/dark selection, animation preference, stable crate/settings/runtime IDs and existing shadows remain.

26 focused theme tests pass, including a new regression for both palettes and all interactive states. The existing ui_shot example rendered the real Font dialog in dark and light through software Vulkan, and both PNGs were visually inspected. The empty document remains a precise white page; no document or image was imported. These are offscreen application renders, not native VM/package/hardware acceptance.

The required cargo xtask ci was invoked. Its first run hit sandbox-denied loopback tests; it was rerun with ordinary-user local sockets available. The rerun passed all 9 steps: fmt, runtime-packaging, clippy, test, i18n-report, doctest, assets, layers and wasm (18 crates). Existing wasm warnings remain; no passing hosted CI is inferred from this local run. No CI workflow is changed. The upstream craftrules repository was unavailable to this account; local AGENTS rules and the Tavru guide were followed.

Full suite packaging and fresh VM editing/round-trip workflows, screen reader, scaling/outputs and physical hardware remain separate acceptance work. Existing Enki/Scribe runtime contracts and upstream notices remain; this is not a broad branding or document-format migration.
