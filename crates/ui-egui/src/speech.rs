use crate::WordApp;
use enki_speech_ui::{Action, VoiceCommand};
use serde_json::json;
pub(crate) fn show(app: &mut WordApp, ctx: &egui::Context) {
    let scope = format!("{}:{}:{:?}", app.session.document_id(), app.session.ime_body_epoch(), app.session.sel);
    let Some(action) = app.speech.show(ctx, &scope, "Insert into document") else { return };
    let result = match action {
        Action::Insert(text) => app.run("text.insert", json!({"text":text})),
        Action::Command(command) => {
            let id = match command {
                VoiceCommand::OpenAssistant => "view.assistantPane",
                VoiceCommand::ZoomIn => "view.zoomIn",
                VoiceCommand::ZoomOut => "view.zoomOut",
                VoiceCommand::Undo => "edit.undo",
                VoiceCommand::Redo => "edit.redo",
                VoiceCommand::Stop => {
                    app.speech.open = false;
                    return;
                }
                VoiceCommand::Next | VoiceCommand::Previous => {
                    if app.session.view.read_mode {
                        crate::canvas::turn_read_page(app, if command == VoiceCommand::Next { 1.0 } else { -1.0 });
                    } else {
                        app.speech.notice("Page navigation is available in Read mode.");
                    }
                    return;
                }
            };
            app.run(id, json!({"value":true}))
        }
    };
    if let Err(e) = result {
        app.status_msg = Some((e.to_string(), ctx.input(|i| i.time)));
    }
}
