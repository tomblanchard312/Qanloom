# Enki speech SDK snapshot

Client, protocol and optional consumer-side desktop controls from tomblanchard312/Enki revision `9c8c1b9ab2a39d10e214c900772fde2945401736`. This follows the existing public client snapshot convention; no daemon, native inference worker, credentials or models are included.

Source logic is unchanged; Rust formatting follows the containing workspace. Manifests spell out their inherited version, edition, license and dependency fields so consumers do not need the Enki workspace. Internal SDK dependency versions are explicit so Git consumers keep the compatibility and speech SDK protocols separate. The owner approved MIT licensing for these three SDK packages; each package includes its MIT license. The daemon, native workers, branding and third-party licenses are unchanged.

Refresh all three directories together from a tested Enki revision. Keep the daemon updated separately for speech protocol versions 4 and 5. Consumer apps pin the Qanloom commit containing this snapshot.
