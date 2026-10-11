//! Automation registry: every interactive element registers a stable id and its screen rect each
//! frame, so agents can click/drag by id (`ui.click {id:"timeline.track.V1.lock"}`) and
//! `ui.inspect` can report the full on-screen widget tree.
//!
//! While the app is driven (`--control`), the registry also reads egui's AccessKit tree at the end
//! of every frame, which describes every widget egui drew. That does two things:
//!
//! * registered elements get what kind of control they are (`role`), whether they are `enabled`,
//!   their `checked` state and current `value`, so an agent can tell a greyed-out button or a
//!   ticked checkbox from the element list alone;
//! * a widget no panel registered still shows up, under a generated id
//!   `auto.<panel or dialog>.<label>` with `discovered: true`. Generated ids follow the layout and
//!   the label, so they are less stable than registered ones; every one is a missing registration
//!   (the `every_button_ui` test lists them). What egui draws by itself gets fixed ids: a window's
//!   close button `window.<title>.close`, a movable window's title bar `window.<title>.titleBar`,
//!   and the number box of a registered slider `<slider id>.value`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use egui::Rect;
use egui::accesskit::{self, Action, Role, Toggled};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Element {
    pub id: String,
    pub label: String,
    pub rect: [f32; 4],
    /// The kind of control: `button`, `checkbox`, `radio`, `comboBox`, `slider`, `number`,
    /// `textInput`, `link`, `color`, `clickable` (a custom widget that takes clicks)…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<&'static str>,
    /// False when the control is greyed out: clicking it does nothing.
    pub enabled: bool,
    /// Checkbox / toggle / selected state, when the control has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Current text or number of a field, slider or value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Not registered by its panel: found in the AccessKit tree, id generated.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub discovered: bool,
}

impl Element {
    pub fn rect(&self) -> Rect {
        Rect::from_min_size(egui::pos2(self.rect[0], self.rect[1]), egui::vec2(self.rect[2], self.rect[3]))
    }
}

/// One interactive widget as AccessKit describes it at the end of a frame.
#[derive(Clone, Debug)]
pub struct A11yNode {
    pub rect: Rect,
    pub role: &'static str,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub value: Option<String>,
    /// Title of the window (dialog) it is in.
    pub window: Option<String>,
}

#[derive(Default)]
pub struct Registry {
    pub elements: Vec<Element>,
    /// Last frame's elements (for lookups from control handlers between frames).
    pub previous: Vec<Element>,
    /// The AccessKit widgets of the last frame, while capturing.
    capture: Option<Arc<Mutex<Vec<A11yNode>>>>,
}

impl Registry {
    pub fn add(&mut self, id: &str, rect: Rect, label: &str) {
        self.elements.push(Element {
            id: id.to_string(),
            label: label.to_string(),
            rect: [rect.min.x, rect.min.y, rect.width(), rect.height()],
            role: None,
            enabled: true,
            checked: None,
            value: None,
            discovered: false,
        });
    }
    pub fn begin_frame(&mut self) {
        let mut els = std::mem::take(&mut self.elements);
        if let Some(c) = &self.capture {
            let nodes = c.lock().unwrap_or_else(PoisonError::into_inner).clone();
            merge(&mut els, &nodes);
        }
        self.previous = els;
    }
    pub fn find(&self, id: &str) -> Option<&Element> {
        self.previous.iter().chain(self.elements.iter()).find(|e| e.id == id)
    }
    /// Elements whose id starts with a prefix.
    pub fn query(&self, prefix: &str) -> Vec<&Element> {
        self.previous.iter().filter(|e| e.id.starts_with(prefix)).collect()
    }
    /// Whether the AccessKit tree is read (see the module docs).
    pub fn capturing(&self) -> bool {
        self.capture.is_some()
    }
    /// Read egui's AccessKit tree from now on (the app is being driven). Idempotent.
    pub fn start_capture(&mut self, ctx: &egui::Context) {
        if self.capture.is_some() {
            return;
        }
        ctx.enable_accesskit();
        let c = Capture::default();
        self.capture = Some(c.nodes.clone());
        ctx.add_plugin(c);
    }
}

/// egui plugin that keeps the interactive nodes of each frame's AccessKit tree.
#[derive(Default)]
struct Capture {
    nodes: Arc<Mutex<Vec<A11yNode>>>,
}

impl egui::Plugin for Capture {
    fn debug_name(&self) -> &'static str {
        "filmcraft-automation"
    }
    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        if let Some(update) = &output.platform_output.accesskit_update {
            *self.nodes.lock().unwrap_or_else(PoisonError::into_inner) = interactive_nodes(update);
        }
    }
}

/// The name of a control's role, or None for nodes that aren't controls (labels, panes, scroll
/// bars, resize handles, windows).
fn role_name(node: &accesskit::Node) -> Option<&'static str> {
    Some(match node.role() {
        Role::Button | Role::DefaultButton => "button",
        Role::CheckBox | Role::Switch => "checkbox",
        Role::RadioButton => "radio",
        Role::ComboBox | Role::EditableComboBox => "comboBox",
        Role::Slider => "slider",
        Role::SpinButton => "number",
        Role::TextInput | Role::MultilineTextInput | Role::SearchInput => "textInput",
        Role::Link => "link",
        Role::ColorWell => "color",
        Role::Tab => "tab",
        Role::MenuItem | Role::MenuItemCheckBox | Role::MenuItemRadio => "menuItem",
        // custom widgets: egui gives anything that senses clicks a node with the Click action
        Role::Unknown if node.supports_action(Action::Click) => "clickable",
        _ => return None,
    })
}

/// The interactive widgets of an AccessKit tree, in tree order.
pub fn interactive_nodes(update: &accesskit::TreeUpdate) -> Vec<A11yNode> {
    let by_id: HashMap<accesskit::NodeId, &accesskit::Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let mut parent: HashMap<accesskit::NodeId, accesskit::NodeId> = HashMap::new();
    for (id, n) in &update.nodes {
        for c in n.children() {
            parent.insert(*c, *id);
        }
    }
    // the nearest titled window above a node (bounded: a malformed tree can't loop forever)
    let window_of = |mut id: accesskit::NodeId| -> Option<String> {
        for _ in 0..256 {
            id = *parent.get(&id)?;
            let n = by_id.get(&id)?;
            if n.role() == Role::Window
                && let Some(l) = n.label().filter(|l| !l.trim().is_empty())
            {
                return Some(l.to_string());
            }
        }
        None
    };
    let mut out = Vec::new();
    for (id, n) in &update.nodes {
        let Some(role) = role_name(n) else { continue };
        let Some(b) = n.bounds() else { continue };
        let rect = Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32));
        if !(rect.width() > 0.0 && rect.height() > 0.0) {
            continue;
        }
        let value = n.value().map(str::to_string).or_else(|| n.numeric_value().filter(|v| v.is_finite()).map(|v| format!("{v}")));
        out.push(A11yNode {
            rect,
            role,
            label: n.label().unwrap_or_default().to_string(),
            enabled: !n.is_disabled(),
            checked: match n.toggled() {
                Some(Toggled::True) => Some(true),
                Some(Toggled::False) => Some(false),
                _ => None,
            },
            value,
            window: window_of(*id),
        });
    }
    out
}

/// Intersection over union of two rects (0 when either is empty).
fn iou(a: Rect, b: Rect) -> f32 {
    let i = a.intersect(b);
    if !(i.width() > 0.0 && i.height() > 0.0) {
        return 0.0;
    }
    let inter = i.area();
    let union = a.area() + b.area() - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// How surely rects `a` and `b` are the same widget (0 = not): IoU, or for rects of similar size
/// (within 4×; a registration often pads the hit rect by a few points) the share of the smaller
/// one inside the larger. A small button in a big container is never the container.
fn same_widget(a: Rect, b: Rect) -> f32 {
    let o = iou(a, b);
    if o >= 0.5 {
        return o;
    }
    let (small, big) = if a.area() <= b.area() { (a.area(), b.area()) } else { (b.area(), a.area()) };
    if small.is_nan() || small <= 0.0 || big / small > 4.0 {
        return 0.0;
    }
    let i = a.intersect(b);
    let inside = if i.width() > 0.0 && i.height() > 0.0 { i.area() / small } else { 0.0 };
    if inside >= 0.8 { inside * 0.5 } else { 0.0 }
}

/// Whether a registered label and an AccessKit label can name the same widget: either is empty
/// (custom widgets have no AccessKit label), or one contains the other, ignoring case, `…` and `✓`.
fn labels_agree(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_start_matches('✓').trim_end_matches(['…', '⏵', '▸']).trim().to_lowercase();
    let (a, b) = (norm(a), norm(b));
    a.is_empty() || b.is_empty() || a.contains(&b) || b.contains(&a)
}

/// `Toggle Track Lock` → `toggleTrackLock` (letters and digits only, at most 48 chars).
pub fn slug(label: &str) -> String {
    let mut out = String::new();
    for (i, w) in label.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).enumerate() {
        let mut cs = w.chars();
        if i == 0 && !w.chars().any(char::is_lowercase) {
            // `OK`, `SDR`: an acronym first is all lower case
            out.extend(w.chars().flat_map(char::to_lowercase));
        } else if let Some(first) = cs.next() {
            if i == 0 {
                out.extend(first.to_lowercase());
            } else {
                out.extend(first.to_uppercase());
            }
            out.extend(cs);
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    out.chars().take(48).collect()
}

/// Enrich the registered elements with what AccessKit says about the same widgets, and add the
/// widgets nobody registered under generated ids.
fn merge(els: &mut Vec<Element>, nodes: &[A11yNode]) {
    // `panel.<id>` rects: the scope of a generated id
    let panels: Vec<(String, Rect)> =
        els.iter().filter_map(|e| e.id.strip_prefix("panel.").filter(|p| !p.contains('.')).map(|p| (slug(p), e.rect()))).collect();
    let registered = els.len();
    let rects: Vec<Rect> = els.iter().map(Element::rect).collect();
    // per element: the node that matches it best (IoU, node index)
    let mut best: Vec<Option<(f32, usize)>> = vec![None; registered];
    let mut matched = vec![false; nodes.len()];
    for (ni, n) in nodes.iter().enumerate() {
        let mut m: Option<(f32, usize)> = None;
        for (ei, r) in rects.iter().enumerate() {
            if !r.intersects(n.rect) {
                continue;
            }
            // a widget over another one (a menu item over a timeline clip) is not that one: when
            // both have a label and they disagree, only a near-identical rect still counts
            let label = els.get(ei).map_or("", |e| e.label.as_str());
            if !labels_agree(label, &n.label) && iou(*r, n.rect) < 0.9 {
                continue;
            }
            let o = same_widget(*r, n.rect);
            if o > 0.0 && m.is_none_or(|(b, _)| o > b) {
                m = Some((o, ei));
            }
        }
        if let Some((o, ei)) = m {
            matched[ni] = true;
            if let Some(slot) = best.get_mut(ei)
                && slot.is_none_or(|(b, _)| o > b)
            {
                *slot = Some((o, ni));
            }
        }
    }
    for (e, b) in els.iter_mut().zip(&best) {
        let Some(n) = b.and_then(|(_, ni)| nodes.get(ni)) else { continue };
        e.role = e.role.or(Some(n.role));
        e.enabled = n.enabled;
        e.checked = e.checked.or(n.checked);
        if e.value.is_none() {
            e.value.clone_from(&n.value);
        }
        if e.label.is_empty() {
            e.label.clone_from(&n.label);
        }
    }
    // registered sliders: their number box is `<slider id>.value`
    let sliders: Vec<(String, Rect)> = els.iter().filter(|e| e.role == Some("slider")).map(|e| (e.id.clone(), e.rect())).collect();
    let is_close = |n: &A11yNode| n.window.is_some() && n.role == "button" && n.label == "Close window";
    // close buttons egui draws on windows, by window title (a movable window's title bar runs through it)
    let closes: Vec<(&str, Rect)> = nodes.iter().filter(|n| is_close(n)).filter_map(|n| Some((n.window.as_deref()?, n.rect))).collect();
    let mut used: HashMap<String, usize> = HashMap::new();
    for (n, _) in nodes.iter().zip(&matched).filter(|(_, m)| !**m) {
        // an unlabeled clickable area around other controls (a menu's background) is no control
        if n.role == "clickable" && n.label.is_empty() && rects.iter().filter(|r| n.rect.contains_rect(**r) && r.area() > 0.0).nth(1).is_some() {
            continue;
        }
        let known = |id: String, label: String| Element {
            id,
            label,
            rect: [n.rect.min.x, n.rect.min.y, n.rect.width(), n.rect.height()],
            role: Some(n.role),
            enabled: n.enabled,
            checked: n.checked,
            value: n.value.clone(),
            discovered: false,
        };
        // egui draws a window's close button itself: a stable id, `window.<title>.close`
        if let Some(w) = n.window.as_deref().filter(|_| is_close(n)) {
            els.push(known(format!("window.{}.close", slug(w)), format!("Close {w}")));
            continue;
        }
        // and the title bar of a movable window (drag it to move the window)
        if let Some(w) = n.window.as_deref().filter(|w| {
            n.role == "clickable"
                && n.label.is_empty()
                && closes.iter().any(|(cw, cr)| cw == w && n.rect.y_range().contains(cr.center().y) && n.rect.width() > 3.0 * cr.width())
        }) {
            els.push(known(format!("window.{}.titleBar", slug(w)), format!("Title bar of {w}")));
            continue;
        }
        // the number box egui draws beside a slider
        if n.role == "number"
            && let Some((sid, _)) = sliders.iter().find(|(_, r)| r.intersect(n.rect).area() >= 0.9 * n.rect.area())
        {
            els.push(known(format!("{sid}.value"), n.label.clone()));
            continue;
        }
        let scope = match &n.window {
            Some(w) => slug(w),
            None => {
                let c = n.rect.center();
                panels.iter().filter(|(_, r)| r.contains(c)).min_by(|a, b| a.1.area().total_cmp(&b.1.area())).map_or_else(|| "app".to_string(), |p| p.0.clone())
            }
        };
        let name = match slug(&n.label) {
            s if s.is_empty() => n.role.to_string(),
            s => s,
        };
        let base = format!("auto.{scope}.{name}");
        let k = used.entry(base.clone()).or_insert(0);
        *k += 1;
        let id = if *k == 1 { base } else { format!("{base}.{k}") };
        els.push(Element {
            id,
            label: n.label.clone(),
            rect: [n.rect.min.x, n.rect.min.y, n.rect.width(), n.rect.height()],
            role: Some(n.role),
            enabled: n.enabled,
            checked: n.checked,
            value: n.value.clone(),
            discovered: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(x: f32, y: f32, w: f32, h: f32, label: &str) -> A11yNode {
        A11yNode {
            rect: Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
            role: "button",
            label: label.into(),
            enabled: false,
            checked: Some(true),
            value: None,
            window: None,
        }
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Toggle Track Lock"), "toggleTrackLock");
        assert_eq!(slug("  Save as New Workspace…"), "saveAsNewWorkspace");
        assert_eq!(slug("1/2"), "12");
        assert_eq!(slug("OK"), "ok");
        assert_eq!(slug("Export SDR"), "exportSDR");
        assert!(labels_agree("Save Preset…", "Save Preset"));
        assert!(labels_agree("", "Video Thumbnails"));
        assert!(!labels_agree("Ocean_Sunset.mp4", "Video Thumbnails"));
        assert_eq!(slug(""), "");
        assert_eq!(slug(&"a ".repeat(200)).chars().count(), 48);
    }

    #[test]
    fn registered_widgets_are_enriched_and_the_rest_discovered() {
        let mut r = Registry::default();
        r.add("panel.timeline", Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 300.0)), "Timeline");
        r.add("timeline.track.V1.locked", Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(18.0, 18.0)), "Toggle Track Lock");
        let mut els = std::mem::take(&mut r.elements);
        let nodes = vec![
            node(10.0, 10.0, 18.0, 18.0, ""),
            node(100.0, 10.0, 40.0, 20.0, "Do It"),
            node(150.0, 10.0, 40.0, 20.0, "Do It"),
            node(600.0, 10.0, 40.0, 20.0, ""),
            A11yNode { window: Some("Audio Gain".into()), ..node(700.0, 10.0, 40.0, 20.0, "OK") },
        ];
        merge(&mut els, &nodes);
        let lock = els.iter().find(|e| e.id == "timeline.track.V1.locked").unwrap();
        assert_eq!((lock.role, lock.enabled, lock.checked, lock.discovered), (Some("button"), false, Some(true), false));
        let ids: Vec<&str> = els.iter().filter(|e| e.discovered).map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["auto.timeline.doIt", "auto.timeline.doIt.2", "auto.app.button", "auto.audioGain.ok"]);
    }

    #[test]
    fn iou_of_empty_and_disjoint_rects_is_zero() {
        let a = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0));
        assert_eq!(iou(a, Rect::from_min_size(egui::pos2(20.0, 0.0), egui::vec2(10.0, 10.0))), 0.0);
        assert_eq!(iou(a, Rect::NOTHING), 0.0);
        assert_eq!(iou(a, a), 1.0);
    }
}
