/// Only these navigation/reversible intents may come from speech. No paths, arguments or shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceCommand {
    OpenAssistant,
    Next,
    Previous,
    ZoomIn,
    ZoomOut,
    Undo,
    Redo,
    Stop,
}
pub fn parse_voice(text: &str) -> Option<VoiceCommand> {
    let text = text.trim().trim_end_matches(['.', '!', '?']).to_lowercase();
    Some(match text.as_str() {
        "open scribe" | "show scribe" | "open assistant" | "show assistant" => VoiceCommand::OpenAssistant,
        "next" | "next page" | "next slide" | "next sheet" => VoiceCommand::Next,
        "previous" | "previous page" | "previous slide" | "previous sheet" => VoiceCommand::Previous,
        "zoom in" => VoiceCommand::ZoomIn,
        "zoom out" => VoiceCommand::ZoomOut,
        "undo" => VoiceCommand::Undo,
        "redo" => VoiceCommand::Redo,
        "stop listening" | "stop recording" => VoiceCommand::Stop,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_safe_commands_are_accepted() {
        assert_eq!(parse_voice(" Zoom in. "), Some(VoiceCommand::ZoomIn));
        for text in ["delete files", "block 1.2.3.4", "undo and delete", "run rm -rf /", "open /etc/passwd", "please zoom in then save"] {
            assert_eq!(parse_voice(text), None);
        }
    }
}
