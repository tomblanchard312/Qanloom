# Releasing WordCraft

The canonical recipe is [craftrules `release/playbook.md`](https://github.com/storytold/craftrules/blob/main/release/playbook.md);
this file has the WordCraft specifics.

1. Bump the version on `main`: `cargo xtask version set 0.2.0` (the one source of truth is
   `[workspace.package] version`; the app shows it in `--version` and About, with the commit from
   `WORDCRAFT_BUILD_SHA`).
2. Merge `main` into `release` and push. `.github/workflows/release.yml` builds:

| Platform | Artifacts | Signing |
|---|---|---|
| macOS | universal `WordCraft.app` in a `.dmg`, universal CLI zip | Developer ID + notarization (`APPLE_*` secrets) |
| Windows | x64, x86 and arm64 `.msi` (one dual-scope package: per-user by default, needs no admin rights; "Everyone on this computer" installs per machine) + portable `.zip` | Azure Trusted Signing (`AZURE_*` secrets) |
| Linux | x86_64 and aarch64 `.AppImage` (+ `.zsync` for AppImageUpdate), `.flatpak` bundle, `.deb`, `.rpm`, `.tar.gz`; Flathub manifest | checksums |
| Linux riscv64 | `.tar.gz` (cross-compiled on `ubuntu-24.04`, glibc >= 2.39; CLI smoke-tested under QEMU) | checksums |
| FreeBSD | x86_64 `.tar.gz` | checksums |
| Web | `wordcraft-web-<version>.zip` (static site: `index.html`, wasm, glue) | none |

3. The run drafts a GitHub Release `WordCraft v<version>` with `SHA256SUMS.txt`; review and publish it.

Signing secrets live in the repository's `release` environment (deployment branch: `release` only),
synced from the release manager's machine (`craftrules/release/signing-setup.md`). Missing secrets
produce unsigned builds with a warning, so forks still build.

Local packaging: `packaging/macos/package.sh --arch universal`, `packaging/windows/package.ps1`,
`packaging/linux/package.sh` (then `packaging/linux/flatpak-bundle.sh` for the Flatpak), `packaging/web/package.sh`.

Local FreeBSD builds use `sh packaging/llama/freebsd-cargo.sh build --release -p wordcraft -p wordcraft-cli`
to apply the local llama.cpp compatibility patch while Cargo runs. The wrapper restores
`Cargo.lock` after the command; the FreeBSD CI job also runs workspace tests and release builds
through it. The shared release workflow has not yet been routed through this wrapper.

Dry run: `gh workflow run release.yml --ref <branch>` builds the unsigned jobs (version, Linux,
Linux riscv64, Flatpak, FreeBSD, web) on any branch; the signing jobs are refused there by the `release`
environment's branch rule, so no draft release is made.


## Private release dependency access

The release caller pins the authenticated `private-release.yml` workflow in
`tomblanchard312/craft-common-actions`. It retains the upstream release recipe
from revision `3e2ac193fdf93009ef6e88ae34fe1c32340ae039`.
`ERIDU_STYLE_DEPLOY_KEY` is a repository secret with read-only access to
`tomblanchard312/eridu-style`. Source jobs use the same pinned checkout as CI.
The key is removed after checkout, and private source and compiled Cargo outputs
are excluded from public Actions caches. FreeBSD receives the checkout through
rsync and resolves it locally in the VM.

To check release dependency access, dispatch the **Release** workflow with
`validate-only` enabled. This resolves the committed lockfile on Linux, Windows,
macOS and FreeBSD. Packaging, signing, registry pushes and release creation are
skipped in this mode. A successful access check does not validate the installers
or signing credentials. Regular release runs retain the existing signing gates
and package formats.
