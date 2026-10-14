//! Reactor FastH3 Omni HTTP client. Work is isolated from the UI thread and the API key never
//! leaves the desktop host except in authenticated requests to Reactor.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use filmcraft_ui_egui::panels::reactor::{Event, Request};
use serde_json::{Value, json};

const BASE: &str = "https://api.reactor.inc";
const MAX_REFERENCE: u64 = 25 * 1024 * 1024;
const MAX_OUTPUT: u64 = 2 * 1024 * 1024 * 1024;

pub fn request(request: Request) -> Receiver<Event> {
    let (tx, rx) = channel();
    let worker_tx = tx.clone();
    let started = std::thread::Builder::new().name("reactor-generation".into()).spawn(move || {
        let result =
            std::panic::catch_unwind(|| generate(request, &worker_tx)).unwrap_or_else(|_| Err("Reactor generation worker failed unexpectedly.".into()));
        let _ = worker_tx.send(Event::Finished(result));
    });
    if let Err(error) = started {
        let _ = tx.send(Event::Finished(Err(format!("Could not start Reactor worker: {error}"))));
    }
    rx
}

fn agent() -> ureq::Agent {
    let provider = std::sync::Arc::new(rustls_rustcrypto::provider());
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .unversioned_rustls_crypto_provider(provider)
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into()
}

fn send_stage(tx: &Sender<Event>, stage: &str) {
    let _ = tx.send(Event::Stage(stage.into()));
}

fn json_response(response: ureq::http::Response<ureq::Body>) -> Result<Value, String> {
    let mut text = String::new();
    response.into_body().as_reader().take(256 * 1024).read_to_string(&mut text).map_err(|error| format!("Could not read Reactor response: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("Invalid Reactor response: {error}"))
}

fn bearer(key: &str) -> String {
    format!("Bearer {key}")
}

fn mime(path: &Path, image: bool) -> Result<&'static str, String> {
    let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    let kind = match (image, ext.as_str()) {
        (true, "png") => "image/png",
        (true, "jpg" | "jpeg") => "image/jpeg",
        (true, "webp") => "image/webp",
        (false, "wav") => "audio/wav",
        (false, "mp3") => "audio/mpeg",
        (false, "m4a") => "audio/mp4",
        (false, "aac") => "audio/aac",
        (false, "ogg" | "opus") => "audio/ogg",
        (false, "flac") => "audio/flac",
        (false, "webm") => "audio/webm",
        _ => return Err(format!("Unsupported Reactor reference file: {}", path.display())),
    };
    Ok(kind)
}

fn upload(agent: &ureq::Agent, key: &str, source: &str, image: bool) -> Result<String, String> {
    let path = Path::new(source);
    let name = path.file_name().and_then(|value| value.to_str()).ok_or("Reference file has no name")?;
    let mime = mime(path, image)?;
    let size = std::fs::metadata(path).map_err(|error| format!("Could not read {source}: {error}"))?.len();
    if size == 0 || size > MAX_REFERENCE {
        return Err(format!("Reference file must be between 1 byte and 25 MiB: {source}"));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("Could not read {source}: {error}"))?;
    if bytes.len() as u64 != size {
        return Err(format!("Reference file changed while reading: {source}"));
    }
    let body = json!({"name": name, "size": size, "mime_type": mime}).to_string();
    let response = agent
        .post(&format!("{BASE}/files"))
        .header("Authorization", bearer(key))
        .header("Content-Type", "application/json")
        .send(body.as_bytes())
        .map_err(|error| format!("Could not create Reactor file: {error}"))?;
    let file = json_response(response)?;
    let id = file["file_id"].as_str().filter(|id| valid_id(id)).ok_or("Reactor returned an invalid file ID")?;
    let url = file["presigned_url"].as_str().filter(|url| url.starts_with("https://")).ok_or("Reactor returned an invalid upload URL")?;
    agent.put(url).header("Content-Type", mime).send(bytes.as_slice()).map_err(|error| format!("Could not upload Reactor reference: {error}"))?;
    Ok(id.into())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn generation_body(request: &Request, images: &[String], audios: &[String]) -> Result<String, String> {
    if request.prompt.trim().is_empty() || request.prompt.len() > 2048 || ![5.875, 8.0, 10.125, 12.25, 14.375].contains(&request.seconds) {
        return Err("Invalid FastH3 Omni prompt or duration.".into());
    }
    if !["16:9", "9:16", "4:3", "1:1"].contains(&request.aspect.as_str()) || images.len() > 9 || audios.len() > 3 || (!audios.is_empty() && images.is_empty()) {
        return Err("Invalid FastH3 Omni aspect or references.".into());
    }
    let mut input = json!({"prompt": request.prompt, "seconds": request.seconds, "aspect": request.aspect});
    if !images.is_empty() {
        input["reference_images"] = json!(images);
    }
    if !audios.is_empty() {
        input["reference_audios"] = json!(audios);
    }
    let body = json!({"model": "reactor/fast-h3-omni", "input": input}).to_string();
    if body.len() > 16 * 1024 {
        return Err("Reactor request exceeds the 16 KiB limit.".into());
    }
    Ok(body)
}

fn generate(request: Request, tx: &Sender<Event>) -> Result<String, String> {
    if request.prompt.trim().is_empty() || request.prompt.len() > 2048 || request.reference_images.len() > 9 || request.reference_audios.len() > 3 {
        return Err("Invalid FastH3 Omni input.".into());
    }
    if !request.reference_audios.is_empty() && request.reference_images.is_empty() {
        return Err("Audio references require at least one reference image.".into());
    }
    let key = crate::reactor_key::read_key()?;
    let client = agent();
    let mut images = Vec::with_capacity(request.reference_images.len());
    let mut audios = Vec::with_capacity(request.reference_audios.len());
    if !request.reference_images.is_empty() || !request.reference_audios.is_empty() {
        send_stage(tx, "Uploading Reactor references…");
    }
    for path in &request.reference_images {
        images.push(upload(&client, &key, path, true)?);
    }
    for path in &request.reference_audios {
        audios.push(upload(&client, &key, path, false)?);
    }
    let body = generation_body(&request, &images, &audios)?;
    send_stage(tx, "Submitting FastH3 Omni generation…");
    let response = client
        .post(&format!("{BASE}/generations"))
        .header("Authorization", bearer(&key))
        .header("Content-Type", "application/json")
        .send(body.as_bytes())
        .map_err(|error| format!("Could not start Reactor generation: {error}"))?;
    let started = json_response(response)?;
    let id = started["id"].as_str().filter(|id| valid_id(id)).ok_or("Reactor returned an invalid generation ID")?;
    let id = id.to_owned();
    for _ in 0..240 {
        send_stage(tx, "FastH3 Omni is generating…");
        std::thread::sleep(Duration::from_secs(5));
        let response = match client.get(&format!("{BASE}/generations/{id}")).header("Authorization", bearer(&key)).call() {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(503)) => continue,
            Err(error) => return Err(format!("Could not check Reactor generation {id}: {error}")),
        };
        let result = json_response(response)?;
        match result["status"].as_str() {
            Some("queued" | "running") => continue,
            Some("failed") => {
                let message = result["error"]["message"].as_str().unwrap_or("Reactor did not provide a reason");
                return Err(format!("Reactor generation {id} failed: {message}"));
            }
            Some("succeeded") => {
                let file = result["files"]
                    .as_array()
                    .and_then(|files| files.iter().find(|file| file["name"] == "output.mp4"))
                    .ok_or("Reactor did not return output.mp4")?;
                let url = file["url"].as_str().filter(|url| url.starts_with("https://")).ok_or("Reactor returned an invalid download URL")?;
                send_stage(tx, "Downloading Reactor clip…");
                download(&client, url, Path::new(&request.output_path), &id)?;
                return Ok(request.output_path);
            }
            _ => return Err("Reactor returned an unknown generation status.".into()),
        }
    }
    Err(format!("Reactor generation {id} is still running. Check your Reactor account; no clip was downloaded."))
}

fn download(agent: &ureq::Agent, url: &str, destination: &Path, id: &str) -> Result<(), String> {
    let parent = destination.parent().filter(|parent| parent.is_dir()).ok_or("Output folder does not exist")?;
    if destination.exists() {
        return Err("Output file already exists. Choose a new file name.".into());
    }
    let name = destination.file_name().and_then(|name| name.to_str()).ok_or("Invalid output file name")?;
    let partial = parent.join(format!(".{name}.reactor-{id}.part"));
    let response = agent.get(url).call().map_err(|error| format!("Could not download Reactor clip: {error}"))?;
    let mut body = response.into_body();
    let mut reader = body.as_reader();
    let mut output =
        std::fs::OpenOptions::new().write(true).create_new(true).open(&partial).map_err(|error| format!("Could not create output file: {error}"))?;
    let copied = std::io::copy(&mut reader.by_ref().take(MAX_OUTPUT + 1), &mut output);
    let result = copied
        .map_err(|error| format!("Could not save Reactor clip: {error}"))
        .and_then(|size| if size > MAX_OUTPUT { Err("Reactor clip exceeds the 2 GiB download limit.".into()) } else { Ok(()) })
        .and_then(|()| output.flush().map_err(|error| format!("Could not flush Reactor clip: {error}")));
    drop(output);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    // Linking within the destination folder creates the final name only if it is still free.
    // A plain rename can overwrite a file created while the clip was downloading.
    if let Err(error) = std::fs::hard_link(&partial, destination) {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("Could not finish Reactor clip: {error}"));
    }
    let _ = std::fs::remove_file(&partial);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_cannot_escape_generation_url_or_output_folder() {
        assert!(valid_id("8c0e7a52-8bed-4d2d-8bc0-56191705124d"));
        assert!(!valid_id("../other"));
        assert!(!valid_id("id?key=value"));
    }

    #[test]
    fn reference_types_are_bounded_to_gateway_formats() {
        assert_eq!(mime(Path::new("picture.WEBP"), true).ok(), Some("image/webp"));
        assert!(mime(Path::new("sound.mp4"), false).is_err());
    }

    #[test]
    fn omni_request_uses_gateway_model_and_input_shape() {
        let request = Request {
            prompt: "Picture 1 waves in the voice of Audio 1".into(),
            seconds: 8.0,
            aspect: "16:9".into(),
            reference_images: vec![],
            reference_audios: vec![],
            output_path: "/tmp/clip.mp4".into(),
        };
        let body = generation_body(&request, &["image-id".into()], &["audio-id".into()]).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["model"], "reactor/fast-h3-omni");
        assert_eq!(parsed["input"]["reference_images"], json!(["image-id"]));
        assert_eq!(parsed["input"]["reference_audios"], json!(["audio-id"]));
        assert!(parsed.get("output_path").is_none());
        assert!(parsed["input"].get("output_path").is_none());
    }
}
