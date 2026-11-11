# Tavru Docs

<img src="assets/app-icon/hicolor/256x256/apps/io.github.tomblanchard312.Nisaba.png" width="96" height="96" alt="Tavru Docs app icon">

Tavru Docs is the TavruOS word processor, written in Rust. Use it to write and
format documents, review tracked changes, and export finished work to PDF.

- **Write and format:** Use styles, tables, references, and mail merge. Open
  and save Word documents (.docx), or open ODT, RTF, HTML, Markdown, and plain text.
- **Review and revise:** Track changes and accept or reject edits. Open **Ask**
  to rewrite, shorten, proofread, or continue text with the local Oqelith
  runtime. Review suggestions before applying them as tracked changes.
- **Use voice features:** Dictation, transcription, translation, and voice
  controls are available as a [preview](docs/VOICE.md).
- **Automate documents:** Convert files from the command line, use the
  [MCP server](docs/mcp.md), or edit VBA modules in the
  [Developer workspace](docs/vba-editor.md). Running supported macros requires
  the [optional .NET runtime setup](vba/README.md). Run only macros you trust.

## Build and run

With Rust and Cargo installed, run these commands from the repository root:

```sh
cargo run --release -p wordcraft -- --sample
cargo run --release -p wordcraft-cli -- convert in.docx out.pdf
```

The first command opens a sample document. The second converts a Word document
to PDF without opening the editor. See [packaging](packaging/) for platform
scripts and the [Ask setup guide](docs/assistant.md) for local inference setup.

Installed packages retain the executable names `nisaba` and `nisaba-cli` and
the application ID `io.github.tomblanchard312.Nisaba` for compatibility.
Settings use the existing `nisaba` directory on Linux and FreeBSD
(`~/.config/nisaba` by default), `~/Library/Application Support/Nisaba` on macOS,
and `%APPDATA%\Nisaba` on Windows.

## Contributing

See [AGENTS.md](AGENTS.md) for contributor rules, [docs](docs/) for technical
guides, and the [roadmap](ROADMAP.md) for planned work.

## License and credits

Tavru Docs is a fork of [WordCraft](https://github.com/storytold/wordcraft) by
the ArtCraft Team and WordCraft contributors. Tavru Docs changes are copyright
(c) 2026 Tom Blanchard. The project is available under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE). See [NOTICE](NOTICE) and
[ATTRIBUTION.md](ATTRIBUTION.md) for bundled material.

The vendored Oqelith client and protocol snapshot retains its own
`UNLICENSED` designation. See its [provenance and terms](vendor/README.md).
