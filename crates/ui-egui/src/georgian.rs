//! Georgian fallback font for the interface. The bundled Latin UI fonts have no Georgian letters,
//! so native builds reuse one installed face (macOS SF Georgian, Windows Sylfaen, Linux Noto Sans
//! Georgian or DejaVu Sans), appended after the other fallbacks. Georgian code points
//! (U+10A0–10FF, U+1C90–1CBF, U+2D00–2D2F) don't overlap the CJK faces, so the order is free.

use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions};

const SAMPLE: &str = "ქართულიაბგდევზთიკლმნოპჟრსტუფღყშჩცძწჭხჯჰ";
const PREFERRED: &[&str] = &["SF Georgian", "Sylfaen", "Noto Sans Georgian", "DejaVu Sans", "Arial Unicode MS", "FreeSans"];

/// One installed face covering Georgian (scanned once per process; none on the web).
fn system_font() -> Option<Arc<FontData>> {
    static FONT: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    FONT.get_or_init(|| {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        filmcraft_text::fonts::scan_system();
        let faces: Vec<_> = filmcraft_text::fonts::all_faces().into_iter().filter(|f| f.info.origin == "system" && !f.info.italic).collect();
        let covers = |f: &filmcraft_text::fonts::Face| f.covers_text(SAMPLE);
        let by_weight = |f: &&Arc<filmcraft_text::fonts::Face>| f.info.weight.abs_diff(400);
        let preferred = PREFERRED.iter().find_map(|name| faces.iter().filter(|f| f.info.family.eq_ignore_ascii_case(name) && covers(f)).min_by_key(by_weight));
        let face = preferred.or_else(|| faces.iter().filter(|f| covers(f)).min_by_key(by_weight))?;
        Some(Arc::new(FontData { font: std::borrow::Cow::Owned(face.data()?), index: face.info.index, tweak: Default::default() }))
    })
    .clone()
}

/// Whether [`install`] has a Georgian face to add. The Georgian interface needs one.
pub(crate) fn available() -> bool {
    system_font().is_some()
}

/// Append the Georgian face as the last fallback of every font family.
pub(crate) fn install(fonts: &mut FontDefinitions) {
    if let Some(font) = system_font() {
        fonts.font_data.insert("system-georgian".into(), font);
        for stack in fonts.families.values_mut() {
            stack.push("system-georgian".into());
        }
    }
}
