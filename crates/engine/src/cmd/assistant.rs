//! Qanloom's writing assistant ([`wordcraft_brand::ASSISTANT_NAME`]): put a model's suggestion
//! into the document as tracked changes.
//!
//! The model runs through the local Enki service (driven by the assistant pane; users only
//! see the assistant's name); the engine only applies the result, so scripts, the CLI and MCP
//! agents get the same behaviour.
//! Suggestions always land as revisions by a separate author, so the writer reviews them with
//! Review › Accept / Reject like any colleague's edits.

use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};
use serde_json::{Value, json};

/// The revision author for applied suggestions.
pub fn author() -> String {
    wordcraft_brand::ASSISTANT_NAME.to_string()
}

/// The largest suggestion accepted in one call (a long chapter, not a whole book).
pub const MAX_TEXT_BYTES: usize = 1 << 20;

/// Where the assistant is opened.
const LOCATION: &str = concat!("Title bar › ", wordcraft_brand::assistant_name!());

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("view.assistantPane", wordcraft_brand::ASSISTANT_NAME, LOCATION, |s, v| {
            let cur = s.view.assistant_pane;
            s.view.assistant_pane = p::bool(v, "value").unwrap_or(!cur);
            Ok(json!({"value": s.view.assistant_pane}))
        })
        .params(r#"{"value"?: bool}"#)
        .pure(),
        CommandSpec::new("assistant.settings", concat!(wordcraft_brand::assistant_name!(), " Settings"), LOCATION, settings)
            .params(r#"{"socket"?: string (legacy setting, ignored by local inference), "model"?: string (GGUF file path; "" for the first available)}"#)
            .pure(),
        CommandSpec::new("assistant.apply", "Apply Suggestion", LOCATION, apply).params(
            r#"{"text": string (plain text; blank lines become paragraphs), "mode"?: "replace" (the selection, default) | "after" (inserted after the selection)}"#,
        ),
    ]
}

/// The local model path and legacy socket setting (retained for saved-preference compatibility).
fn settings(s: &mut Session, v: &Value) -> CmdResult {
    let clean = |t: &str| t.trim().chars().filter(|c| !c.is_control()).take(1024).collect::<String>();
    if let Some(t) = p::str(v, "socket") {
        s.prefs.assistant_socket = clean(t);
    }
    if let Some(t) = p::str(v, "model") {
        s.prefs.assistant_model = clean(t);
    }
    Ok(json!({"socket": s.prefs.assistant_socket, "model": s.prefs.assistant_model}))
}

fn apply(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").ok_or_else(|| CmdError::Params("text: the suggestion to apply".into()))?;
    if text.len() > MAX_TEXT_BYTES {
        return Err(CmdError::Params(format!("text: at most {} KiB", MAX_TEXT_BYTES >> 10)));
    }
    let text = text.replace("\r\n", "\n");
    if text.trim().is_empty() {
        return Err(CmdError::Params("text: the suggestion is empty".into()));
    }
    let after = match p::str(v, "mode") {
        None | Some("replace") => false,
        Some("after") => true,
        Some(_) => return Err(CmdError::Params(r#"mode: "replace" | "after""#.into())),
    };
    if after {
        let (_, end) = s.sel.ordered();
        s.sel = Selection::caret(end);
    }
    let (track, writer) = (s.doc.settings.track_changes, std::mem::replace(&mut s.author, author()));
    s.doc.settings.track_changes = true;
    let r = (|| {
        if !after && rewrite_in_place(s, &text)? {
            return Ok(json!({}));
        }
        s.run("edit.pasteSpecial", &json!({"as": "text", "text": text}))
    })();
    s.doc.settings.track_changes = track;
    s.author = writer;
    r.map(|_| json!({"applied": true, "author": author(), "mode": if after { "after" } else { "replace" }}))
}

/// Keep paragraph properties and untouched runs when a rewrite retains its paragraph structure.
fn rewrite_in_place(s: &mut Session, text: &str) -> Result<bool, CmdError> {
    use super::tools::{Op, lcs_ops};
    let (a, b) = s.sel.ordered();
    let paths = s.doc.paths_between(&a, &b);
    let lines: Vec<_> = text.split('\n').collect();
    let lines = if lines.len() != paths.len() { text.split("\n\n").collect::<Vec<_>>() } else { lines };
    if paths.len() != lines.len() {
        // shortcut: restructuring multiple paragraphs needs Insert until paragraph mapping is available.
        if paths.len() > 1 {
            return Err(CmdError::Failed(
                "This suggestion changes the paragraph structure. Use Insert, or request a rewrite that keeps the original paragraphs.".into(),
            ));
        }
        return Ok(false);
    }
    let mut changes = Vec::new();
    let mut budget = 4_000_000;
    for (path, new) in paths.iter().zip(lines) {
        let Some(p) = s.doc.para(a.story, path) else { return Ok(false) };
        let from = if *path == a.path { a.off } else { 0 };
        let to = if *path == b.path { b.off } else { p.len() };
        let old = p.text.get(from..to).ok_or_else(|| CmdError::Failed("Invalid suggestion selection".into()))?;
        if p.run_ranges().any(|(_, c)| c.del.is_some()) {
            return Err(CmdError::Failed("Accept or reject existing tracked deletions before applying another rewrite.".into()));
        }
        if old.len() > MAX_TEXT_BYTES {
            return Err(CmdError::Params("The selected paragraph is too long to rewrite safely.".into()));
        }
        let old_chars: Vec<_> = old.chars().collect();
        let new_chars: Vec<_> = new.chars().collect();
        let mut offset = from;
        let mut next = 0;
        let mut old_index = 0;
        let mut start = None;
        let mut replacement = String::new();
        for op in lcs_ops(&old_chars, &new_chars, &mut budget).into_iter().chain(std::iter::once(Op::Same)) {
            match op {
                Op::Same => {
                    if let Some(begin) = start.take() {
                        let mut anchor = a.clone();
                        anchor.path = path.clone();
                        anchor.off = begin;
                        let mut focus = anchor.clone();
                        focus.off = offset;
                        changes.push((Selection { anchor, focus }, std::mem::take(&mut replacement)));
                    }
                    offset += old_chars.get(old_index).map_or(0, |c| c.len_utf8());
                    old_index += 1;
                    next += 1;
                }
                Op::Del => {
                    start.get_or_insert(offset);
                    offset += old_chars.get(old_index).map_or(0, |c| c.len_utf8());
                    old_index += 1;
                }
                Op::Ins => {
                    start.get_or_insert(offset);
                    if let Some(c) = new_chars.get(next) {
                        replacement.push(*c);
                    }
                    next += 1;
                }
            }
        }
    }
    for (selection, replacement) in changes.into_iter().rev() {
        s.sel = selection;
        super::paste_text(s, &replacement)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::RevisionKind;

    fn session(text: &str) -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": text})).unwrap();
        s
    }

    #[test]
    fn a_rewrite_replaces_the_selection_as_tracked_changes_by_the_assistant() {
        let mut s = session("The quick brown fox.");
        s.run("select.text", &json!({"text": "quick brown"})).unwrap();
        s.run("assistant.apply", &json!({"text": "slow grey"})).unwrap();
        assert!(!s.doc.settings.track_changes, "the writer's Track Changes setting is kept");
        assert_ne!(s.author, author(), "the writer's name is put back");
        let mine: Vec<_> = s.doc.revisions.iter().filter(|r| r.author == author()).map(|r| r.kind).collect();
        assert!(mine.iter().any(|k| matches!(k, RevisionKind::Insert)), "{mine:?}");
        assert!(mine.iter().any(|k| matches!(k, RevisionKind::Delete)), "{mine:?}");
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "The quick brown fox.");
    }

    #[test]
    fn accepting_gives_the_suggestion_and_undo_is_one_step() {
        let mut s = session("Hello wrld.");
        s.run("select.text", &json!({"text": "wrld"})).unwrap();
        s.run("assistant.apply", &json!({"text": "world"})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Hello wrld.", "one undo removes the whole suggestion");
        assert!(s.doc.revisions.iter().all(|r| r.author != author()) || !s.doc.settings.track_changes);
        s.run("edit.redo", &json!({})).unwrap();
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Hello world.");
    }

    #[test]
    fn after_mode_keeps_the_selection_and_paragraphs_split() {
        let mut s = session("First.");
        s.run("select.text", &json!({"text": "First."})).unwrap();
        s.run("assistant.apply", &json!({"text": "\nSecond.\nThird.", "mode": "after"})).unwrap();
        s.run("review.acceptAll", &json!({})).unwrap();
        let body = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(body.starts_with("First."), "{body:?}");
        assert!(body.contains("Second.") && body.contains("Third."), "{body:?}");
    }

    #[test]
    fn multi_paragraph_suggestions_keep_breaks_and_unchanged_formatting() {
        let mut s = session("Heading\rFirst wrng paragraph.\rSecond wrng paragraph.");
        s.run("select.text", &json!({"text":"First"})).unwrap();
        s.run("format.bold", &json!({})).unwrap();
        let paths = s.doc.para_paths(wordcraft_doc::StoryRef::Body);
        s.doc.para_mut(wordcraft_doc::StoryRef::Body, &paths[0]).unwrap().props.style = Some("Heading1".into());
        let props: Vec<_> = paths.iter().map(|path| s.doc.para(wordcraft_doc::StoryRef::Body, path).unwrap().props.clone()).collect();
        s.run("select.all", &json!({})).unwrap();
        s.run("assistant.apply", &json!({"text":"Heading\nFirst correct paragraph.\nSecond correct paragraph."})).unwrap();
        s.run("review.acceptAll", &json!({})).unwrap();
        let paths = s.doc.para_paths(wordcraft_doc::StoryRef::Body);
        assert_eq!(paths.len(), 3, "Accept All must preserve paragraph breaks");
        assert_eq!(paths.iter().map(|path| s.doc.para(wordcraft_doc::StoryRef::Body, path).unwrap().props.clone()).collect::<Vec<_>>(), props);
        let first = s.doc.para(wordcraft_doc::StoryRef::Body, &paths[1]).unwrap();
        assert!(first.run_ranges().any(|(r, c)| r.start == 0 && c.bold == Some(true)), "unchanged bold text must stay bold");
        assert_eq!(first.text, "First correct paragraph.");
        let reopened = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        let saved_paths = reopened.para_paths(wordcraft_doc::StoryRef::Body);
        assert_eq!(saved_paths.len(), 3);
        assert_eq!(reopened.para(wordcraft_doc::StoryRef::Body, &saved_paths[0]).unwrap().props.style.as_deref(), Some("Heading1"));
        assert!(
            reopened.para(wordcraft_doc::StoryRef::Body, &saved_paths[1]).unwrap().run_ranges().any(|(r, c)| r.start == 0 && c.bold == Some(true))
        );

        s.run("edit.undo", &json!({})).unwrap();
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Heading\nFirst wrng paragraph.\nSecond wrng paragraph.");
    }

    #[test]
    fn tracked_paste_replacement_does_not_delete_new_paragraph_marks() {
        let mut s = session("One\rTwo\rThree");
        s.run("select.all", &json!({})).unwrap();
        s.doc.settings.track_changes = true;
        s.run("edit.pasteSpecial", &json!({"as":"text","text":"New one\nNew two"})).unwrap();
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "New one\nNew two");
    }

    #[test]
    fn unicode_and_partial_selections_preserve_surrounding_text() {
        let mut s = session("Avant café mal écrit après.");
        s.run("select.text", &json!({"text":"café mal écrit"})).unwrap();
        s.run("assistant.apply", &json!({"text":"café bien écrit"})).unwrap();
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Avant café bien écrit après.");
    }

    #[test]
    fn a_flattened_reply_cannot_replace_multiple_paragraphs() {
        let mut s = session("One\rTwo\rThree");
        s.run("select.all", &json!({})).unwrap();
        assert!(s.run("assistant.apply", &json!({"text":"One Two Three"})).is_err());
        assert_eq!(s.doc.para_paths(wordcraft_doc::StoryRef::Body).len(), 3);
        assert!(s.doc.revisions.is_empty());
    }

    #[test]
    fn bad_params_are_refused_without_changing_anything() {
        let mut s = session("Keep me.");
        for v in [json!({}), json!({"text": "  "}), json!({"text": "x", "mode": "sideways"}), json!({"text": "x".repeat(MAX_TEXT_BYTES + 1)})] {
            assert!(s.run("assistant.apply", &v).is_err(), "{v}");
        }
        assert!(s.doc.revisions.is_empty());
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Keep me.");
    }

    #[test]
    fn settings_are_kept_clean() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let r = s.run("assistant.settings", &json!({"socket": " /run/user/1/sharedai/ipc.sock\n", "model": "llama-3.2-3b"})).unwrap();
        assert_eq!(r, json!({"socket": "/run/user/1/sharedai/ipc.sock", "model": "llama-3.2-3b"}));
        assert_eq!(s.run("assistant.settings", &json!({})).unwrap(), r);
    }

    #[test]
    fn the_pane_toggles() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        assert_eq!(s.run("view.assistantPane", &json!({})).unwrap(), json!({"value": true}));
        assert_eq!(s.run("view.assistantPane", &json!({"value": false})).unwrap(), json!({"value": false}));
    }
}
