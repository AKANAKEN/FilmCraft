//! FastH3 Omni generation dialog. The desktop host owns the credential and HTTP work.

use std::sync::mpsc::{Receiver, TryRecvError};

use egui::Context;
use filmcraft_project::BinId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::FilmcraftApp;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub open: bool,
    pub prompt: String,
    pub seconds: f64,
    pub aspect: String,
    pub reference_images: Vec<String>,
    pub reference_audios: Vec<String>,
    pub message: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            open: false,
            prompt: String::new(),
            seconds: 8.0,
            aspect: "16:9".into(),
            reference_images: vec![],
            reference_audios: vec![],
            message: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub prompt: String,
    pub seconds: f64,
    pub aspect: String,
    #[serde(default)]
    pub reference_images: Vec<String>,
    #[serde(default)]
    pub reference_audios: Vec<String>,
    pub output_path: String,
}

pub enum Event {
    Stage(String),
    Finished(Result<String, String>),
}

pub(crate) struct Running {
    pub receiver: Receiver<Event>,
    pub project_name: String,
    pub project_path: Option<String>,
    pub bin: BinId,
}

pub fn route(app: &mut FilmcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "reactor.generate" {
        return None;
    }
    if params.as_object().is_none_or(|p| p.is_empty()) {
        app.ui.reactor.open = true;
        return Some(Ok(json!({"dialog": "reactor"})));
    }
    let request: Request = match serde_json::from_value(params.clone()) {
        Ok(request) => request,
        Err(error) => return Some(Err(format!("Invalid Reactor generation request: {error}"))),
    };
    Some(start(app, request).map(|()| json!({"started": true})))
}

fn start(app: &mut FilmcraftApp, request: Request) -> Result<(), String> {
    if app.reactor_running.is_some() {
        return Err("A Reactor generation is already running.".into());
    }
    if request.prompt.trim().is_empty() || request.prompt.len() > 2048 {
        return Err("Enter a prompt of at most 2048 UTF-8 bytes.".into());
    }
    if ![5.875, 8.0, 10.125, 12.25, 14.375].contains(&request.seconds) {
        return Err("Choose a supported FastH3 Omni duration.".into());
    }
    if !["16:9", "9:16", "4:3", "1:1"].contains(&request.aspect.as_str()) {
        return Err("Choose a supported aspect ratio.".into());
    }
    if request.reference_images.len() > 9 || request.reference_audios.len() > 3 || (!request.reference_audios.is_empty() && request.reference_images.is_empty())
    {
        return Err("Use up to 9 images and 3 audio clips; audio requires an image.".into());
    }
    if !std::path::Path::new(&request.output_path).is_absolute() || !request.output_path.to_ascii_lowercase().ends_with(".mp4") {
        return Err("Choose an absolute MP4 output path.".into());
    }
    let output = std::path::Path::new(&request.output_path);
    if output.exists() {
        return Err("Output file already exists. Choose a new file name.".into());
    }
    if !output.parent().is_some_and(std::path::Path::is_dir) {
        return Err("Output folder does not exist.".into());
    }
    let Some(generate) = app.hooks.reactor_generate.as_mut() else {
        return Err("Reactor generation is available in the desktop app.".into());
    };
    app.reactor_running = Some(Running {
        receiver: generate(request),
        project_name: app.session.project.name.clone(),
        project_path: app.session.path.clone(),
        bin: app.import_bin(),
    });
    app.ui.reactor.message = "Starting Reactor generation…".into();
    app.ui.reactor.open = true;
    Ok(())
}

pub fn poll(app: &mut FilmcraftApp, ctx: &Context) {
    let next = app.reactor_running.as_ref().map(|job| job.receiver.try_recv());
    match next {
        Some(Ok(Event::Stage(message))) => app.ui.reactor.message = message,
        Some(Ok(Event::Finished(result))) => {
            let running = app.reactor_running.take();
            match result {
                Err(error) => app.ui.reactor.message = error,
                Ok(path) => {
                    if let Some(running) = running {
                        if app.session.project.name != running.project_name || app.session.path != running.project_path {
                            app.ui.reactor.message = format!("Clip saved to {path}. Import it into the current project when ready.");
                        } else {
                            match app.session.execute("file.import", json!({"paths": [path], "bin": running.bin.0})) {
                                Ok(value) if value["items"].as_array().is_some_and(|items| !items.is_empty()) => {
                                    app.ui.reactor.message = "Reactor clip generated and imported into the Project panel.".into();
                                }
                                Ok(value) => app.ui.reactor.message = format!("Clip saved, but import failed: {}", value["errors"]),
                                Err(error) => app.ui.reactor.message = format!("Clip saved, but import failed: {error}"),
                            }
                        }
                    }
                }
            }
            app.ui.status = app.ui.reactor.message.clone();
        }
        Some(Err(TryRecvError::Disconnected)) => {
            app.reactor_running = None;
            app.ui.reactor.message = "Reactor worker stopped unexpectedly.".into();
            app.ui.status = app.ui.reactor.message.clone();
        }
        _ => {}
    }
    if app.reactor_running.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

pub fn show(app: &mut FilmcraftApp, ctx: &Context) {
    if !app.ui.reactor.open {
        return;
    }
    let mut open = true;
    egui::Window::new(tl!("Generate with Reactor FastH3 Omni")).open(&mut open).collapsible(false).resizable(true).default_width(440.0).show(ctx, |ui| {
        ui.label(tl!("Describe the video and sound"));
        let prompt = ui.add(egui::TextEdit::multiline(&mut app.ui.reactor.prompt).desired_rows(4).desired_width(f32::INFINITY));
        app.auto.add("reactor.prompt", prompt.rect, "Reactor prompt");
        ui.label(format!("{} / 2048 bytes", app.ui.reactor.prompt.len()));
        ui.horizontal(|ui| {
            ui.label(tl!("Duration"));
            egui::ComboBox::from_id_salt("reactor.seconds").selected_text(format!("{} s", app.ui.reactor.seconds)).show_ui(ui, |ui| {
                for seconds in [5.875, 8.0, 10.125, 12.25, 14.375] {
                    let response = ui.selectable_value(&mut app.ui.reactor.seconds, seconds, format!("{seconds} s"));
                    app.auto.add(&format!("reactor.seconds.{seconds}"), response.rect, &format!("{seconds} seconds"));
                }
            });
            ui.label(tl!("Aspect"));
            egui::ComboBox::from_id_salt("reactor.aspect").selected_text(&app.ui.reactor.aspect).show_ui(ui, |ui| {
                for aspect in ["16:9", "9:16", "4:3", "1:1"] {
                    let response = ui.selectable_value(&mut app.ui.reactor.aspect, aspect.into(), aspect);
                    app.auto.add(&format!("reactor.aspect.{aspect}"), response.rect, aspect);
                }
            });
        });
        ui.separator();
        let images = ui.button(format!("{} ({}/9)…", tl!("Reference images"), app.ui.reactor.reference_images.len()));
        app.auto.add("reactor.images", images.rect, "Choose reference images");
        if images.clicked()
            && let Some(pick) = app.hooks.pick_files.as_mut()
        {
            app.ui.reactor.reference_images = pick(&["png", "jpg", "jpeg", "webp"]);
        }
        for (index, path) in app.ui.reactor.reference_images.iter().enumerate() {
            let name = std::path::Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path);
            ui.label(format!("{} {}: {name}", tl!("Picture"), index + 1));
        }
        let audios = ui.button(format!("{} ({}/3)…", tl!("Reference audio"), app.ui.reactor.reference_audios.len()));
        app.auto.add("reactor.audios", audios.rect, "Choose reference audio");
        if audios.clicked()
            && let Some(pick) = app.hooks.pick_files.as_mut()
        {
            app.ui.reactor.reference_audios = pick(&["wav", "mp3", "m4a", "aac", "ogg", "opus", "flac", "webm"]);
        }
        for (index, path) in app.ui.reactor.reference_audios.iter().enumerate() {
            let name = std::path::Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path);
            ui.label(format!("{} {}: {name}", tl!("Audio"), index + 1));
        }
        ui.label(tl!("Use Picture 1 and Audio 1 in the prompt to refer to the first selected files."));
        if !app.ui.reactor.message.is_empty() {
            ui.label(&app.ui.reactor.message);
        }
        ui.separator();
        let enabled = app.hooks.reactor_generate.is_some() && app.reactor_running.is_none();
        let generate = ui.add_enabled(enabled, egui::Button::new(tl!("Generate and import…")));
        app.auto.add("reactor.start", generate.rect, "Generate and import Reactor clip");
        if generate.clicked() {
            if app.ui.reactor.prompt.trim().is_empty() || app.ui.reactor.prompt.len() > 2048 {
                app.ui.reactor.message = "Enter a prompt of at most 2048 UTF-8 bytes.".into();
            } else if let Some(output_path) = app.hooks.pick_save_as.as_mut().and_then(|pick| pick("MP4 video", &["mp4"], "Reactor clip.mp4")) {
                let draft = &app.ui.reactor;
                let request = Request {
                    prompt: draft.prompt.clone(),
                    seconds: draft.seconds,
                    aspect: draft.aspect.clone(),
                    reference_images: draft.reference_images.clone(),
                    reference_audios: draft.reference_audios.clone(),
                    output_path,
                };
                if let Err(error) = start(app, request) {
                    app.ui.reactor.message = error;
                }
            }
        }
        if app.hooks.reactor_generate.is_none() {
            ui.label(tl!("Generation is available in the desktop app after adding a Reactor API key in Settings."));
        }
    });
    app.ui.reactor.open = open;
}
