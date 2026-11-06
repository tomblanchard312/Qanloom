# Enki, the writing assistant

> **Last updated:** 2026-11-05 · **Change:** Renamed Enki to Enki and adopted the Eridu chat styling.

Enki runs writing requests on this computer through the local Enki daemon (`sharedai-daemon`). It suggests rewrites and new text that can be applied as tracked changes. The title bar and writing pane use the official folded blue E. See [asset attribution](../ATTRIBUTION.md).

## Using it

1. Choose **Enki** in the title bar. The first time, Qanloom lists its local model files in the background ("Starting Enki…").
2. With no model yet, the pane says that the local assistant needs text-generation setup. In **Settings**, choose **Download Phi-4 mini** (about 2.5 GB, MIT). That is the default. Qanloom checks the pinned SHA-256 and keeps the file in its models folder (`~/.local/share/nisaba/models`, private to the user), then selects it. **Choose Model File…** and the other catalog models under **Settings** are still available. If Phi-4 mini is already there, Enki selects it ahead of other files unless a model was chosen already.
3. Select some text and choose **Improve**, **Shorten**, **Fix grammar** or **More formal**, or type a request and choose **Send** (Ctrl+Enter, or Cmd+Enter on macOS). **Continue** writes a concise next paragraph after the selection, or uses text before the document caret when nothing is selected. On an empty body paragraph it can use the previous paragraph.
4. The final answer streams into the pane; internal reasoning prefixes are withheld. **Stop** ends generation. Stopped, failed, empty or length-limited results cannot be applied; visible draft text can still be copied.
5. **Replace** swaps the selection for the suggestion, **Insert** adds it after the selection, **Copy** puts it on the clipboard and **Discard** drops it.

Replace targets the text selected when the request was made, even if only the selection moved. Any document edit or switch makes the suggestion stale and requires a fresh request. Enki never falls back to replacing a different selection. Selections over 24,000 characters are refused rather than silently truncated. Requests are limited to 4,000 characters.

Model selection, downloads and file import are grouped under **Settings**. Routine writing controls show the task rather than model names. An explicitly selected existing external model path is retained during discovery. Failures read plainly ("Enki is busy. Try again in a moment.", "That's more text than Enki can read at once. Select less."); the technical detail goes to Qanloom's log.

## Downloading models

Like FileExplorer, **Download a model** lists fixed models with their size, license and Hugging Face page. Phi-4 mini is the default. Qwen3 4B, Gemma 3 4B, and Llama 3.2 3B are also pinned to exact revisions, sizes and SHA-256 hashes. Downloading shows progress and supports Cancel; only a verified file is installed and selected. Incomplete downloads restart from the beginning. Catalog models are checked again before inference.

For a gated repository, accept its access terms on Hugging Face and enter a read token. It is held only for the download, never saved; `HF_TOKEN` is also supported. Flatpak builds allow network access for these downloads. Inference stays local. Llama 4 is not in the small-model catalog.

## Local inference

Models stay in the app's data folder. Each request is sent to the per-user Enki daemon, which Qanloom starts when it is not already listening (`sharedai-daemon` beside the app, in `SHAREDAI_DAEMON`, in `SHAREDAI_BIN_DIR`, or on `PATH`, with `sharedai-worker` beside the daemon). The daemon registers the selected file and streams the reply. Stop cancels that request. Loading can take the first few minutes of a request; each later chunk waits up to a minute.

Old socket preferences remain readable for compatibility. The socket is the daemon's own endpoint, not a setting Enki edits. Previously registered models outside Qanloom's models folder must be chosen again. Browser builds keep the assistant disabled.

## For scripts and agents

| Command | Params | Does |
|---|---|---|
| `view.assistantPane` | `{"value"?: bool}` | Shows or hides the pane |
| `assistant.settings` | `{"socket"?: string, "model"?: string}` | Sets and returns the GGUF model path and legacy socket preference (ignored) |
| `assistant.apply` | `{"text": string, "mode"?: "replace" \| "after"}` | Puts `text` into the document as tracked changes by "Enki", in one undo step. The writer's Track Changes setting and author name are left as they were. |
| `ui.assistant.downloadModel` | `{"model": "qwen3-4b" \| "gemma3-4b" \| "phi4-mini" \| "llama3.2-3b"}` | Downloads and selects a pinned model |
| `ui.assistant.addModel` | `{"path": string}` | Adds a GGUF model file, as Choose Model File… does (in the background) |
| `ui.assistant.ask` | `{"action"?: "improve" \| "shorten" \| "grammar" \| "formal" \| "continue", "prompt"?: string}` | Starts the same writing request as the pane |
| `ui.assistant.state` | `{}` | Returns busy/ready state, visible reply and status |
| `ui.assistant.apply` | `{"mode"?: "replace" \| "after"}` | Applies a complete suggestion to its captured, unchanged document target |
| `ui.assistant.stop` / `ui.assistant.discard` | `{}` | Stops generation or discards the suggestion |

An MCP agent with its own model can call `assistant.apply` to propose edits for review the same way.

## Validation on 2026-10-26

Nine Enki UI/state tests and six engine application tests pass. Real local inference corrected
subject/verb agreement while preserving shipment details; Shorten retained the date, capacity,
hours, cost and volunteer caveat. Suggestions were applied as Enki revisions; rejecting the
grammar rewrite restored the exact source. Continue worked with no selection and preserved the
original as a separate paragraph. The first continuation was overly verbose; the prompt now asks
for concise prose matched to the source. Style and instruction adherence still need broader evaluation.

Headless dark-theme screenshots were inspected. Clippy and the 18-crate WASM check passed.
These are local headless UI/runtime checks, not native GUI or cross-platform acceptance.

## How it's built

- `crates/sharedai/src/lib.rs`: validates the selected GGUF, starts or connects to the Enki daemon, registers the model and streams chat; `crates/sharedai/src/local.rs` validates model files.
- `crates/engine/src/cmd/assistant.rs`: settings and tracked-change application.
- `crates/ui-egui/src/assistant_pane.rs`: local model discovery, verified downloads, import, daemon-backed streaming and cancellation on worker threads.

The daemon keeps a loaded model between requests. Not yet: GPU acceleration, or conversation history beyond the messages Enki sends.

## Revision history

| Date | Change |
|---|---|
| 2026-11-05 | Renamed the writing assistant to Enki and updated the chat controls. |
| 2026-10-28 | Scribe and Developer ask the Enki daemon for chat and completion. Qanloom no longer links llama.cpp. |
| 2026-10-26 | Embedded the approved blue S/reed-tip icon, cached per UI context; inspected light/dark headless renders. |
| 2026-10-26 | Guarded stale/partial suggestions and oversized selections; separated source data in prompts, added concise caret continuation, grouped model setup in Settings and verified real writing/review flows. |
| 2026-10-25 | Replaced the daemon dependency with in-process CPU GGUF inference and a checksum-pinned Hugging Face download catalog. |
