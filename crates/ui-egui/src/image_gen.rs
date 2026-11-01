//! Generate an image, a logo, or a generative expand through the SharedAI daemon.
//!
//! The call runs on a worker thread. Each frame polls the result and asks for another frame
//! while it is still running. An expand replaces the selected picture.

use serde_json::json;
use std::sync::mpsc::{self, Receiver};

use crate::WordApp;

const APP: &str = "nisaba";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Task {
    Image,
    Logo,
    Expand,
}

pub struct State {
    open: bool,
    task: Task,
    prompt: String,
    negative: String,
    model: String,
    models: Vec<String>,
    width: String,
    height: String,
    steps: String,
    expand_left: String,
    expand_right: String,
    expand_top: String,
    expand_bottom: String,
    /// Media key to replace when an expand finishes, if that picture is still selected.
    replace: Option<String>,
    status: String,
    /// Download bytes or sampling steps for the job in `rx`.
    meter: Option<Meter>,
    rx: Option<Receiver<Outcome>>,
    started: Option<std::time::Instant>,
    #[cfg(not(target_arch = "wasm32"))]
    cancel: Option<sharedai_client::Cancellation>,
}

#[derive(Clone, Copy)]
enum Meter {
    Download { done: u64, total: u64 },
    Draw { step: u32, total: u32 },
}

enum Outcome {
    Models(Result<Vec<String>, String>),
    Default { model: String, side: u32, steps: u32 },
    Picture(Result<Generated, String>),
    Progress { done: u64, total: u64 },
    Step { step: u32, total: u32 },
    Downloaded { registered: bool, message: String },
}

struct Generated {
    png: Vec<u8>,
    width: u32,
    height: u32,
    seed: i64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            task: Task::Image,
            prompt: String::new(),
            negative: String::new(),
            model: initial_model(),
            models: Vec::new(),
            width: initial_side(),
            height: initial_side(),
            steps: initial_steps(),
            expand_left: "128".into(),
            expand_right: "128".into(),
            expand_top: "128".into(),
            expand_bottom: "128".into(),
            replace: None,
            status: String::new(),
            meter: None,
            rx: None,
            started: None,
            #[cfg(not(target_arch = "wasm32"))]
            cancel: None,
        }
    }
}

pub fn open(app: &mut WordApp, task: &str) {
    app.image_gen.open = true;
    app.image_gen.task = match task {
        "logo" => Task::Logo,
        "expand" => Task::Expand,
        _ => Task::Image,
    };
    if app.image_gen.models.is_empty() && app.image_gen.rx.is_none() {
        refresh(app);
    }
}

pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    if let Some(outcome) = poll(&mut app.image_gen, ctx) {
        apply(app, outcome);
    }
    if !app.image_gen.open {
        return;
    }
    let mut open = true;
    let title = match app.image_gen.task {
        Task::Image => tl!("Generate image"),
        Task::Logo => tl!("Generate logo"),
        Task::Expand => tl!("Expand picture"),
    };
    let busy = app.image_gen.rx.is_some();
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0)).open(&mut open).show(
        ctx,
        |ui| {
            ui.set_min_width(440.0);
            body(app, ui);
        },
    );
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    } else if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.image_gen.open = false;
    }
}

pub fn cancel(state: &mut State) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(cancel) = &state.cancel {
        cancel.cancel();
        state.status = "Cancelling…".into();
        return true;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = state;
    false
}

fn body(app: &mut WordApp, ui: &mut egui::Ui) {
    let busy = app.image_gen.rx.is_some();
    if busy || !app.image_gen.status.is_empty() {
        let status = app.image_gen.status.clone();
        let meter = app.image_gen.meter;
        notice(ui, busy, &status, meter.as_ref());
    }
    ui.add_enabled_ui(!busy, |ui| {
        fields(app, ui);
    });
    if busy {
        if let Some(started) = app.image_gen.started {
            let elapsed = started.elapsed().as_secs();
            ui.label(format!("Elapsed: {}m {:02}s", elapsed / 60, elapsed % 60));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if app.image_gen.cancel.is_some() && ui.button("Cancel").clicked() {
            let _ = app.run("ui.imageGen.cancel", json!({}));
        }
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(!busy, egui::Button::new(tl!("Generate"))).clicked() {
            generate(app);
        }
        if ui.add_enabled(!busy, egui::Button::new(tl!("Close"))).clicked() {
            app.image_gen.open = false;
        }
    });
}

fn notice(ui: &mut egui::Ui, busy: bool, status: &str, meter: Option<&Meter>) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            if busy {
                ui.add(egui::Spinner::new().size(20.0));
            }
            let text = if status.is_empty() { "Working…" } else { status };
            ui.label(egui::RichText::new(tl!(text)).strong());
        });
        match meter {
            Some(Meter::Download { done, total }) => {
                let frac = *done as f32 / (*total).max(1) as f32;
                ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).show_percentage());
            }
            Some(Meter::Draw { step, total }) => {
                let frac = *step as f32 / (*total).max(1) as f32;
                ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).text(format!("{step} / {total}")));
            }
            None => {}
        }
    });
}

fn fields(app: &mut WordApp, ui: &mut egui::Ui) {
    let busy = app.image_gen.rx.is_some();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.image_gen.task, Task::Image, tl!("Image"));
        ui.selectable_value(&mut app.image_gen.task, Task::Logo, tl!("Logo"));
        ui.selectable_value(&mut app.image_gen.task, Task::Expand, tl!("Expand"));
    });
    ui.label(match app.image_gen.task {
        Task::Image => tl!("Describe the picture. To draw words, put them in quotes."),
        Task::Logo => tl!("Describe the mark. The logo wording is added for you."),
        Task::Expand => tl!("Describe what to paint in the new margin. The selected picture stays."),
    });
    if app.image_gen.task == Task::Expand && app.selected_picture_media().is_none() {
        ui.label(tl!("Select a picture to expand."));
    }
    ui.add(egui::TextEdit::multiline(&mut app.image_gen.prompt).desired_rows(3).hint_text(tl!("a heron in profile")));
    ui.label(tl!("Negative prompt"));
    ui.add(egui::TextEdit::singleline(&mut app.image_gen.negative).hint_text(tl!("optional")));
    model_row(app, ui, busy);
    if app.image_gen.task == Task::Expand {
        ui.label(tl!("Margins, in pixels"));
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Left"));
            ui.add(egui::TextEdit::singleline(&mut app.image_gen.expand_left).desired_width(48.0)).labelled_by(label.id);
            let label = ui.label(tl!("Right"));
            ui.add(egui::TextEdit::singleline(&mut app.image_gen.expand_right).desired_width(48.0)).labelled_by(label.id);
            let label = ui.label(tl!("Top"));
            ui.add(egui::TextEdit::singleline(&mut app.image_gen.expand_top).desired_width(48.0)).labelled_by(label.id);
            let label = ui.label(tl!("Bottom"));
            ui.add(egui::TextEdit::singleline(&mut app.image_gen.expand_bottom).desired_width(48.0)).labelled_by(label.id);
        });
    } else {
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Width"));
            ui.add(egui::TextEdit::singleline(&mut app.image_gen.width).desired_width(64.0)).labelled_by(label.id);
            let label = ui.label(tl!("Height"));
            ui.add(egui::TextEdit::singleline(&mut app.image_gen.height).desired_width(64.0)).labelled_by(label.id);
            let label = ui.label(tl!("Steps"));
            ui.add(egui::TextEdit::singleline(&mut app.image_gen.steps).desired_width(48.0)).labelled_by(label.id);
        });
        ui.label(tl!("Enki chooses the image engine and sampling settings for the selected processor."));
    }
}

fn model_row(app: &mut WordApp, ui: &mut egui::Ui, busy: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    let previous = app.image_gen.model.clone();
    ui.label(tl!("Enki selects image generation behind the scenes."));
    #[cfg(not(target_arch = "wasm32"))]
    if !app.image_gen.model.is_empty()
        && !sharedai_client::image_model_present(app.image_gen.model.trim())
        && ui.add_enabled(!busy, egui::Button::new(tl!("Set up image generation"))).clicked()
    {
        start_download(app);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                if ui.button("CPU default").clicked() {
                    app.image_gen.model.clear();
                    refresh_for(app, Some(true));
                }
                if ui.button("GPU default").clicked() {
                    app.image_gen.model.clear();
                    refresh_for(app, Some(false));
                }
                if ui.button("Use Enki default").clicked() {
                    app.image_gen.model.clear();
                    refresh(app);
                }
            });
        });
        if app.image_gen.model != previous
            && let Some(p) = sharedai_client::image_profile(&app.image_gen.model)
        {
            app.image_gen.width = p.side.to_string();
            app.image_gen.height = p.side.to_string();
            app.image_gen.steps = p.steps.to_string();
        }
    }
}

fn generate(app: &mut WordApp) {
    #[cfg(target_arch = "wasm32")]
    {
        app.image_gen.status = "Image generation runs in the desktop app.".into();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(request) = request(app) else { return };
        let (tx, rx) = mpsc::channel();
        app.image_gen.rx = Some(rx);
        app.image_gen.meter = None;
        app.image_gen.status = "Waiting for Enki / loading the image model…".into();
        app.image_gen.started = Some(std::time::Instant::now());
        let cancel = sharedai_client::Cancellation::new();
        app.image_gen.cancel = Some(cancel.clone());
        std::thread::spawn(move || {
            let result = sharedai_client::generate_image_cancellable(APP, request, &cancel, |step, total| {
                let _ = tx.send(Outcome::Step { step, total });
            })
            .map(|picture| Generated { png: picture.png, width: picture.width, height: picture.height, seed: picture.seed })
            .map_err(|e| e.to_string());
            let _ = tx.send(Outcome::Picture(result));
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn request(app: &mut WordApp) -> Option<sharedai_client::ImageRequest> {
    if app.image_gen.model.trim().is_empty() {
        app.image_gen.status = "Choose a processor default to set up image generation.".into();
        return None;
    }
    if sharedai_client::image_profile(app.image_gen.model.trim()).is_some() && !sharedai_client::image_model_present(app.image_gen.model.trim()) {
        app.image_gen.status = "Set up image generation first.".into();
        return None;
    }
    if app.image_gen.prompt.trim().is_empty() {
        app.image_gen.status = "Write a prompt first.".into();
        return None;
    }
    let model = app.image_gen.model.trim().to_string();
    let prompt = app.image_gen.prompt.trim().to_string();
    let negative = app.image_gen.negative.trim().to_string();
    let mut request = match app.image_gen.task {
        Task::Image | Task::Logo => {
            app.image_gen.replace = None;
            let width = match side(&app.image_gen.width) {
                Ok(n) => n,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            let height = match side(&app.image_gen.height) {
                Ok(n) => n,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            let steps = match whole(&app.image_gen.steps, "steps") {
                Ok(n) => n,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            let mut request = match app.image_gen.task {
                Task::Logo => sharedai_client::ImageRequest::logo(model, prompt),
                _ => sharedai_client::ImageRequest::image(model, prompt),
            };
            request.width = width;
            request.height = height;
            request.steps = steps;
            request
        }
        Task::Expand => {
            let left = match margin(&app.image_gen.expand_left) {
                Ok(n) => n,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            let right = match margin(&app.image_gen.expand_right) {
                Ok(n) => n,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            let top = match margin(&app.image_gen.expand_top) {
                Ok(n) => n,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            let bottom = match margin(&app.image_gen.expand_bottom) {
                Ok(n) => n,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            if left + right + top + bottom == 0 {
                app.image_gen.status = "An expand needs a margin on at least one side.".into();
                return None;
            }
            let Some(media) = app.selected_picture_media() else {
                app.image_gen.status = "Select a picture to expand.".into();
                return None;
            };
            let Some(bytes) = app.session.doc.media.get(&media).cloned() else {
                app.image_gen.status = "The selected picture has no image data.".into();
                return None;
            };
            let png = match fit_png(bytes.as_slice(), left, right, top, bottom) {
                Ok(png) => png,
                Err(e) => {
                    app.image_gen.status = e;
                    return None;
                }
            };
            app.image_gen.replace = Some(media);
            let mut request = sharedai_client::ImageRequest::expand(model, prompt, png);
            request.expand_left = left;
            request.expand_right = right;
            request.expand_top = top;
            request.expand_bottom = bottom;
            request
        }
    };
    request.negative = negative;
    Some(request)
}

fn refresh(app: &mut WordApp) {
    refresh_for(app, None);
}

fn refresh_for(app: &mut WordApp, cpu: Option<bool>) {
    #[cfg(target_arch = "wasm32")]
    let _ = cpu;
    #[cfg(target_arch = "wasm32")]
    {
        app.image_gen.status = "Image generation runs in the desktop app.".into();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.image_gen.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        app.image_gen.rx = Some(rx);
        app.image_gen.meter = None;
        app.image_gen.status = "Looking for diffusion models…".into();
        std::thread::spawn(move || {
            match cpu.map_or_else(|| sharedai_client::recommended_image_profile(APP), |cpu| sharedai_client::recommended_image_profile_for(APP, cpu))
            {
                Ok(profile) => {
                    let _ = tx.send(Outcome::Default { model: profile.model, side: profile.side, steps: profile.steps });
                }
                Err(error) => {
                    let _ = tx.send(Outcome::Models(Err(format!("Update or start Enki to choose an image default: {error}"))));
                    return;
                }
            }
            let result = sharedai_client::diffusion_models(APP).map_err(|e| e.to_string());
            let _ = tx.send(Outcome::Models(result));
        });
    }
}

fn apply(app: &mut WordApp, outcome: Outcome) {
    if !app.image_gen.open {
        return;
    }
    match outcome {
        Outcome::Models(Ok(ids)) => {
            app.image_gen.models = ids;
            if app.image_gen.model.is_empty()
                && let Some(first) = app.image_gen.models.first()
            {
                app.image_gen.model = first.clone();
            }
            app.image_gen.status = if app.image_gen.models.is_empty() {
                "Image generation needs setup. Use Set up image generation or configure it in Enki.".into()
            } else {
                String::new()
            };
        }
        Outcome::Models(Err(e)) => app.image_gen.status = e,
        Outcome::Progress { .. } | Outcome::Step { .. } | Outcome::Default { .. } => {}
        Outcome::Downloaded { registered, message } => {
            app.image_gen.status = message;
            if registered {
                refresh(app);
            }
        }
        Outcome::Picture(Ok(picture)) => {
            let data = wordcraft_engine::cmd::insert::base64_encode(&picture.png);
            let replace = app.image_gen.replace.clone();
            let result = if replace.is_some() && replace == app.selected_picture_media() {
                app.run("picture.change", json!({"data": data}))
            } else {
                app.run("insert.picture", json!({"data": data}))
            };
            app.image_gen.replace = None;
            match result {
                Ok(_) => {
                    app.image_gen.status = format!("Placed a {}×{} image (seed {}).", picture.width, picture.height, picture.seed);
                }
                Err(e) => app.image_gen.status = e,
            }
        }
        Outcome::Picture(Err(e)) => {
            app.image_gen.replace = None;
            app.image_gen.status = e;
        }
    }
}

fn poll(state: &mut State, ctx: &egui::Context) -> Option<Outcome> {
    let rx = state.rx.as_ref()?;
    loop {
        match rx.try_recv() {
            Ok(Outcome::Default { model, side, steps }) => {
                if state.model.is_empty() {
                    state.model = model;
                    state.width = side.to_string();
                    state.height = side.to_string();
                    state.steps = steps.to_string();
                }
            }
            Ok(Outcome::Progress { done, total }) => {
                state.status = download_status(done, total);
                state.meter = Some(Meter::Download { done, total });
                ctx.request_repaint();
            }
            Ok(Outcome::Step { step, total }) => {
                state.status = if step == total { "Finishing the PNG…".into() } else { format!("Drawing, step {step} of {total}.") };
                state.meter = Some(Meter::Draw { step, total });
                ctx.request_repaint();
            }
            Ok(outcome) => {
                state.rx = None;
                state.started = None;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    state.cancel = None;
                }
                state.meter = None;
                return Some(outcome);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                return None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                state.rx = None;
                state.started = None;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    state.cancel = None;
                }
                state.meter = None;
                return Some(Outcome::Picture(Err("image generation stopped".into())));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_download(app: &mut WordApp) {
    if app.image_gen.rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    app.image_gen.rx = Some(rx);
    app.image_gen.meter = None;
    app.image_gen.status = "Downloading image model…".into();
    app.image_gen.started = Some(std::time::Instant::now());
    let model = app.image_gen.model.clone();
    std::thread::spawn(move || {
        let result = sharedai_client::download_image_model(&model, |done, total| {
            let _ = tx.send(Outcome::Progress { done, total });
        });
        let outcome = match result {
            Ok(report) => Outcome::Downloaded {
                registered: report.registered,
                message: if report.registered {
                    "Image generation is ready.".into()
                } else {
                    "Image generation setup is incomplete. Check Enki and retry.".into()
                },
            },
            Err(e) => Outcome::Downloaded { registered: false, message: e.to_string() },
        };
        let _ = tx.send(outcome);
    });
}

fn initial_model() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

fn initial_side() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        "512".into()
    }
    #[cfg(target_arch = "wasm32")]
    {
        "512".into()
    }
}

fn initial_steps() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        "8".into()
    }
    #[cfg(target_arch = "wasm32")]
    {
        "20".into()
    }
}

fn download_status(done: u64, total: u64) -> String {
    fn gb(n: u64) -> String {
        format!("{:.1} GB", n as f64 / 1_000_000_000.0)
    }
    format!("Downloading image model… {} of {}", gb(done), gb(total))
}

#[cfg(not(target_arch = "wasm32"))]
fn side(text: &str) -> Result<u32, String> {
    let n = whole(text, "size")?;
    if n != 0 && (!(64..=1024).contains(&n) || n % 8 != 0) {
        return Err("width and height must be 64..=1024 and a multiple of 8".into());
    }
    Ok(n)
}

#[cfg(not(target_arch = "wasm32"))]
fn margin(text: &str) -> Result<u32, String> {
    let n = whole(text, "margin")?;
    if n > sharedai_client::limits::MAX_EXPAND_PIXELS {
        return Err("each margin must be 512 pixels or less".into());
    }
    Ok(n)
}

#[cfg(not(target_arch = "wasm32"))]
fn whole(text: &str, name: &str) -> Result<u32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    text.parse().map_err(|_| format!("{name} must be a whole number"))
}

/// Scale and re-encode the selected picture so the expanded canvas fits the protocol.
#[cfg(not(target_arch = "wasm32"))]
fn fit_png(bytes: &[u8], left: u32, right: u32, top: u32, bottom: u32) -> Result<Vec<u8>, String> {
    use sharedai_client::limits;
    let img = image::load_from_memory(bytes).map_err(|e| format!("cannot read the picture: {e}"))?;
    let max_w = limits::MAX_IMAGE_SIDE.saturating_sub(left.saturating_add(right));
    let max_h = limits::MAX_IMAGE_SIDE.saturating_sub(top.saturating_add(bottom));
    if max_w < 1 || max_h < 1 {
        return Err("the margins leave no room for the picture".into());
    }
    let (mut w, mut h) = (img.width().min(max_w), img.height().min(max_h));
    if img.width() > max_w || img.height() > max_h {
        let scale = (max_w as f32 / img.width() as f32).min(max_h as f32 / img.height() as f32);
        w = ((img.width() as f32) * scale).floor().max(1.0) as u32;
        h = ((img.height() as f32) * scale).floor().max(1.0) as u32;
    }
    let mut current = img;
    loop {
        let canvas_w = w.saturating_add(left).saturating_add(right);
        let canvas_h = h.saturating_add(top).saturating_add(bottom);
        if canvas_w < limits::MIN_IMAGE_SIDE || canvas_h < limits::MIN_IMAGE_SIDE {
            return Err("the expanded image must be at least 64 pixels on a side".into());
        }
        let resized = if current.width() == w && current.height() == h {
            current.clone()
        } else {
            current.resize_exact(w, h, image::imageops::FilterType::Triangle)
        };
        let mut png = Vec::new();
        resized.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| format!("cannot encode the picture: {e}"))?;
        if png.len() <= limits::MAX_SOURCE_PNG_BYTES {
            return Ok(png);
        }
        if w <= 64 && h <= 64 {
            return Err("the picture is too large to expand".into());
        }
        w = (w * 3 / 4).max(1);
        h = (h * 3 / 4).max(1);
        current = resized;
    }
}

/// Image warming is owned by the daemon; retained for existing desktop entry points.
#[cfg(not(target_arch = "wasm32"))]
pub fn warm_default_image_model() {
    // Start Enki as before; only the daemon chooses and warms the resident image default.
    let _ = std::thread::Builder::new().name("enki-start".into()).spawn(|| {
        let result = wordcraft_sharedai::endpoint("").and_then(|endpoint| wordcraft_sharedai::ensure_running(&endpoint, Some(&endpoint)));
        if let Err(error) = result {
            log::warn!("Enki could not be started: {error}");
        }
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod image_progress_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn pending_image_job_stays_with_its_document_window() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("First")), crate::Services::default());
        let ctx = egui::Context::default();
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        // This state-only regression has no renderer to consume texture uploads.
        frame.textures_delta.clear();
        let first = app.switch_windows_entries()[0].0;
        let (tx, rx) = mpsc::channel();
        app.image_gen.prompt = "First document image".into();
        app.image_gen.rx = Some(rx);
        app.run("file.new", json!({})).unwrap();
        assert!(app.image_gen.rx.is_none());
        assert!(app.image_gen.prompt.is_empty());
        tx.send(Outcome::Step { step: 2, total: 8 }).unwrap();
        assert!(app.enter_window(first));
        assert_eq!(app.image_gen.prompt, "First document image");
        assert!(poll(&mut app.image_gen, &egui::Context::default()).is_none());
        assert!(matches!(app.image_gen.meter, Some(Meter::Draw { step: 2, total: 8 })));
    }

    #[test]
    fn cpu_recommendation_sets_sampling_and_refresh_preserves_a_selection() {
        let (tx, rx) = mpsc::channel();
        let mut state = State { rx: Some(rx), ..Default::default() };
        let ctx = egui::Context::default();
        tx.send(Outcome::Default { model: sharedai_client::CPU_IMAGE_MODEL_ID.into(), side: 512, steps: 8 }).unwrap();
        assert!(poll(&mut state, &ctx).is_none());
        assert_eq!((state.model.as_str(), state.width.as_str(), state.steps.as_str()), (sharedai_client::CPU_IMAGE_MODEL_ID, "512", "8"));
        tx.send(Outcome::Default { model: sharedai_client::GPU_IMAGE_MODEL_ID.into(), side: 1024, steps: 50 }).unwrap();
        assert!(poll(&mut state, &ctx).is_none());
        assert_eq!(state.model, sharedai_client::CPU_IMAGE_MODEL_ID);
        assert_eq!(state.steps, "8");
    }

    #[test]
    #[ignore = "requires the installed Enki daemon and a suitable image GPU"]
    fn installed_daemon_defaults_arrive_through_both_buttons() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.image_gen.open = true;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(760.0, 720.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                if let Some(outcome) = poll(&mut app.image_gen, &ctx) {
                    apply(app, outcome);
                }
                body(app, ui);
            },
            app,
        );
        h.run_steps(2);
        for (button, model, side, steps) in
            [("CPU default", sharedai_client::CPU_IMAGE_MODEL_ID, "512", "8"), ("GPU default", sharedai_client::GPU_IMAGE_MODEL_ID, "1024", "50")]
        {
            h.get_by_label(button).click();
            h.run_steps(2);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            while h.state().image_gen.rx.is_some() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
                h.run_steps(1);
            }
            assert!(h.state().image_gen.rx.is_none(), "default query stalled");
            assert_eq!(h.state().image_gen.model, model, "{}", h.state().image_gen.status);
            assert_eq!(h.state().image_gen.width, side);
            assert_eq!(h.state().image_gen.steps, steps);
        }
    }

    #[test]
    fn busy_box_keeps_default_and_progress_until_terminal_result() {
        let (tx, rx) = mpsc::channel();
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.image_gen.open = true;
        app.image_gen.rx = Some(rx);
        app.image_gen.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(65));
        app.image_gen.cancel = Some(sharedai_client::Cancellation::new());
        tx.send(Outcome::Default { model: sharedai_client::GPU_IMAGE_MODEL_ID.into(), side: 1024, steps: 50 }).unwrap();
        tx.send(Outcome::Step { step: 3, total: 50 }).unwrap();
        let builder = egui_kittest::Harness::builder().with_size(egui::vec2(760.0, 720.0));
        // The renderer is only needed for the optional screenshot below.
        let builder = if crate::tests::gpu_available() { builder.wgpu() } else { builder };
        let mut h = builder.build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                if let Some(outcome) = poll(&mut app.image_gen, &ctx) {
                    apply(app, outcome);
                }
                body(app, ui);
            },
            app,
        );
        h.run_steps(3);
        assert_eq!(h.state().image_gen.model, sharedai_client::GPU_IMAGE_MODEL_ID);
        assert_eq!(h.state().image_gen.steps, "50");
        assert!(h.state().image_gen.rx.is_some());
        assert!(h.query_by_label("Drawing, step 3 of 50.").is_some());
        assert!(h.query_by_label_contains("Elapsed: 1m").is_some());
        if let Some(dir) = std::env::var_os("ENKI_IMAGE_UI_SHOTS").filter(|_| crate::tests::gpu_available()) {
            h.render().unwrap().save(std::path::PathBuf::from(dir).join("wordcraft-Qanloom.png")).unwrap();
        }
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert_eq!(h.state().image_gen.status, "Cancelling…");
        tx.send(Outcome::Picture(Err("cancelled".into()))).unwrap();
        h.run_steps(2);
        assert!(h.state().image_gen.rx.is_none());
        assert!(h.state().image_gen.started.is_none());
        assert!(h.state().image_gen.cancel.is_none());
    }
}
