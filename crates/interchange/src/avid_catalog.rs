//! Development bridge from the standalone Avid reader's JSON catalog.
//!
//! No AVB/PMR/MDB binary decoding or file I/O. Imported clips are metadata-only offline
//! placeholders until FilmCraft supports their independently located media components.
use std::collections::{BTreeMap, BTreeSet};

use filmcraft_media::{MediaInfo, MediaKind};
use filmcraft_project::{BinEntry, BinId, Interpretation, ItemKind, Label, MediaClip, MediaRef, Project};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use serde::Serialize;
use serde_json::Value;

use crate::{Error, Result};

mod native;
pub use native::{native_document, plan_native};

pub const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_BINS: usize = 1024;
const MAX_CLIPS: usize = 100_000;
const MAX_MOBS: usize = 200_000;
const MAX_TEXT: usize = 4096;
/// All supported source fields are copied here, not linked to the AVB/report.
pub const SOURCE_KEY: &str = "filmcraft:avid-source";
/// Sub-bin holding masters an imported bin's sequences reference without placing them. Kept out
/// of the bin's own listing so it still reflects what Avid shows.
pub const REFERENCED_BIN: &str = "Referenced Sources";

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    bins: Vec<BinPlan>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
struct BinPlan {
    pub name: String,
    pub uid: String,
    pub clips: Vec<ClipPlan>,
    /// Masters the bin's sequences reference without placing them in the bin. Not bin contents;
    /// FilmCraft still needs them as items because media binds to an item, never to a timeline
    /// clip. Populated for sequence transfers only.
    pub referenced: Vec<ClipPlan>,
    /// Placed subclips: bin contents in their own right, restricted to a range of a master this
    /// bin also carries. Avid records that range in attributes, not in the component.
    pub subclips: Vec<SubclipPlan>,
    pub compositions: usize,
}
#[derive(Clone, Debug, Serialize)]
struct SubclipPlan {
    pub name: String,
    /// Identity of the master this subclip restricts. Must be imported for the bin.
    pub parent: [u8; 32],
    pub start: Tick,
    pub duration: Tick,
    pub evidence: Value,
}
#[derive(Clone, Debug, Serialize)]
struct ClipPlan {
    pub name: String,
    pub duration: Tick,
    pub evidence: Value,
    pub source_descriptors: Vec<Value>,
    pub picture_rate_evidence: Option<Value>,
}
#[derive(Debug, Serialize)]
pub struct ImportedBin {
    pub bin: BinId,
    pub name: String,
    pub clips: usize,
    /// Sequence sources imported into the sub-bin, not part of the Avid bin's own contents.
    pub referenced: usize,
    /// Subclips imported as bin contents, each restricting one of this bin's masters.
    pub subclips: usize,
    pub compositions: usize,
}

/// Requested transfer stages. Media/sequence stages must be verified by the engine before commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMode {
    Metadata,
    Media,
    Sequences,
}
fn bad(message: impl Into<String>) -> Error {
    Error::Other(format!("Avid catalog: {}", message.into()))
}
fn array<'a>(v: &'a Value, name: &str) -> Result<&'a Vec<Value>> {
    v.get(name).and_then(Value::as_array).ok_or_else(|| bad(format!("missing or invalid {name}")))
}
fn text<'a>(v: &'a Value, name: &str) -> Result<&'a str> {
    let s = v.get(name).and_then(Value::as_str).ok_or_else(|| bad(format!("missing or invalid {name}")))?;
    if s.len() > MAX_TEXT || s.contains('\0') {
        return Err(bad(format!("invalid {name} text")));
    }
    Ok(s)
}
/// The catalog contract carries binary IDs with explicit byte order, never display URNs.
/// Normalize the material-number fields for duplicate detection; preserve raw evidence.
fn identity_key(m: &Value) -> Result<Vec<u8>> {
    let id = m.get("id").ok_or_else(|| bad("missing source identity"))?;
    let raw = text(id, "raw")?;
    let order = text(id, "byte_order")?;
    if !matches!(raw.len(), 24 | 64) || !raw.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(order, "little" | "big") {
        return Err(bad("invalid source identity"));
    }
    let mut bytes = raw
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).map_err(|_| bad("invalid identity hex"))?;
            u8::from_str_radix(digits, 16).map_err(|_| bad("invalid identity hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    if bytes.len() == 12 {
        if order != "little" || bytes.get(..4) != Some(&[42, 0, 0, 0]) {
            return Err(bad("unsupported short source identity"));
        }
    } else if order == "little" {
        for range in [16..20, 20..22, 22..24] {
            bytes.get_mut(range).ok_or_else(|| bad("invalid identity width"))?.reverse();
        }
    }
    Ok(bytes)
}
/// The one source identity a subclip restricts, with the offset recorded on its component.
/// More than one, or none, is not a plain subclip.
fn single_source(m: &Value) -> Option<([u8; 32], i64)> {
    fn walk(v: &Value, out: &mut Vec<([u8; 32], i64)>, depth: usize) {
        if depth > 24 || out.len() > 256 {
            return;
        }
        match v {
            Value::Object(o) => {
                if o.get("class").and_then(Value::as_str) == Some("SCLP")
                    && let Some(id) = o.get("source_id").and_then(|i| full_identity(i).ok())
                {
                    out.push((id, o.get("start").and_then(Value::as_i64).unwrap_or(0)));
                }
                for x in o.values() {
                    walk(x, out, depth + 1);
                }
            }
            Value::Array(a) => {
                for x in a {
                    walk(x, out, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(m.get("tracks")?, &mut found, 0);
    let (id, start) = *found.first()?;
    // Every track must restrict the same master at the same offset for one range to describe it.
    found.iter().all(|(i, s)| *i == id && *s == start).then_some((id, start))
}

/// Where a subclip's range actually lives. Avid writes it two ways, and reading either alone is
/// wrong somewhere: with `_START`/`_END` present the component says "the whole master" and the
/// mob's length is the master's, while elsewhere the offset is on the component and the duration
/// is the mob's length. Measured against a fixture whose in and out points were set deliberately.
fn subclip_range(m: &Value, component_start: i64) -> Result<(Tick, Tick)> {
    let attrs = m.get("attributes");
    let start_attr = attrs.and_then(|a| a.get("_START")).and_then(Value::as_i64).filter(|n| *n >= 0);
    let end_attr = attrs.and_then(|a| a.get("_END")).and_then(Value::as_i64).filter(|n| *n >= 0);
    let (offset, span) = match (start_attr, end_attr) {
        // Attributes present: the offset is `_START` and the span `_END` - `_START`. A zero
        // `_START` means the component carries the offset instead.
        (Some(a), end) => (if a > 0 { a } else { component_start }, end.filter(|e| *e > a).map(|e| e - a)),
        (None, _) => (component_start, None),
    };
    if offset < 0 {
        return Err(bad("invalid subclip offset"));
    }
    let rate = m.get("rate").ok_or_else(|| bad("subclip has no rate"))?;
    let to_tick = |frames: i64| -> Result<Tick> { Ok(duration(&serde_json::json!({"length":frames,"rate":rate}))?.0) };
    let span = match span {
        Some(n) if n > 0 => n,
        // Without a usable span the mob's own length is the duration — but only where that length
        // describes the subclip. In the attribute form it is the master's, so an offset with no
        // span is refused rather than silently stretched to the end of the master.
        _ if offset > 0 && start_attr.is_some_and(|a| a > 0) => return Err(bad("it records a start without a usable end")),
        _ => m.get("length").and_then(Value::as_i64).filter(|n| *n > 0).ok_or_else(|| bad("it has no usable range"))?,
    };
    Ok((to_tick(offset)?, to_tick(span)?))
}
/// Decide one placed usage-2 composition. Anything that is not a plain range of one available
/// master is reported by name rather than approximated.
fn subclip(m: &Value, available: &BTreeMap<[u8; 32], Tick>) -> Result<(SubclipPlan, bool)> {
    if m.get("live") == Some(&Value::Bool(false)) {
        return Err(bad("the bin marks it inactive"));
    }
    let (parent, component_start) = single_source(m).ok_or_else(|| bad("it does not restrict exactly one master"))?;
    let extent = *available.get(&parent).ok_or_else(|| bad("its master is not in this bin"))?;
    let name = text(m, "name")?.to_owned();
    let (start, mut duration) = subclip_range(m, component_start)?;
    if duration <= Tick::ZERO {
        return Err(bad("its range is empty"));
    }
    // A subclip cannot exceed what it restricts. Recorded ranges do run past a master's end, by a
    // frame or by dozens, so bound it here rather than build an item reaching past its media.
    let end = start.0.checked_add(duration.0).ok_or_else(|| bad("its range overflows"))?;
    let clamped = end > extent.0;
    if clamped {
        if start >= extent {
            return Err(bad("it starts after the end of its master"));
        }
        duration = Tick(extent.0 - start.0);
    }
    Ok((SubclipPlan { name, parent, start, duration, evidence: m.clone() }, clamped))
}
fn membership(m: &Value) -> Result<bool> {
    m.get("user_placed").and_then(Value::as_bool).ok_or_else(|| bad("missing master bin membership"))
}

/// Canonical full identity for media binding; short-only evidence cannot authorize a file.
pub fn full_identity(id: &Value) -> Result<[u8; 32]> {
    identity_key(&serde_json::json!({"id":id}))?.try_into().map_err(|_| bad("media binding requires a full source identity"))
}

/// Compare recorded edit units with an independently corroborated clock. Recognized nominal
/// NTSC labels are explicit pairs, never a floating-point tolerance or blanket substitution.
/// This is not permission to substitute rates without opening the identified picture source.
pub fn picture_rate_matches(recorded: &Value, actual: filmcraft_time::FrameRate) -> bool {
    let Some(n) = recorded["rate"]["numerator"].as_i64().filter(|n| *n > 0) else { return false };
    let Some(d) = recorded["rate"]["denominator"].as_i64().filter(|d| *d > 0) else { return false };
    actual.num > 0
        && actual.den > 0
        && (i128::from(n) * i128::from(actual.den) == i128::from(d) * i128::from(actual.num)
            || [(23976, 1000, 24000), (2997, 100, 30000), (47952, 1000, 48000), (4795, 100, 48000), (5994, 100, 60000), (11988, 100, 120000)].iter().any(
                |&(rn, rd, an)| {
                    filmcraft_time::FrameRate::new(n, d) == filmcraft_time::FrameRate::new(rn, rd) && actual == filmcraft_time::FrameRate::new(an, 1001)
                },
            ))
}
/// Integer conversion with sub-tick flooring; original count/rate stays in evidence.
fn duration(v: &Value) -> Result<(Tick, bool)> {
    let count = v.get("length").and_then(Value::as_i64).filter(|n| *n >= 0).ok_or_else(|| bad("invalid clip length"))?;
    let rate = v.get("rate").ok_or_else(|| bad("missing clip rate"))?;
    let num = rate.get("numerator").and_then(Value::as_i64).filter(|n| *n > 0).ok_or_else(|| bad("invalid rate numerator"))?;
    let den = rate.get("denominator").and_then(Value::as_i64).filter(|n| *n > 0).ok_or_else(|| bad("invalid rate denominator"))?;
    let scaled =
        i128::from(count).checked_mul(i128::from(den)).and_then(|x| x.checked_mul(i128::from(TICKS_PER_SECOND))).ok_or_else(|| bad("timing overflow"))?;
    let quantized = scaled % i128::from(num) != 0;
    let ticks = i64::try_from(scaled / i128::from(num)).map_err(|_| bad("clip duration exceeds tick range"))?;
    if ticks > Tick::MAX.0 {
        return Err(bad("clip duration exceeds supported tick range"));
    }
    Ok((Tick(ticks), quantized))
}
/// Derive the bin label portably, including catalog paths recorded on Windows.
fn bin_name(path: &str) -> Result<String> {
    let file = path.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = file.get(..file.len().saturating_sub(4)).filter(|_| file.get(file.len().saturating_sub(4)..).is_some_and(|x| x.eq_ignore_ascii_case(".avb")));
    let name = stem.filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("bin path must end in a nonempty .avb filename"))?;
    Ok(name.to_owned())
}
/// Validate the complete transfer before starting a project transaction.
pub fn plan(bytes: &[u8]) -> Result<Plan> {
    plan_scoped(bytes, TransferMode::Metadata)
}

/// Folder/MDB scan inventories are not project-transfer inputs. Let serde validate their JSON
/// syntax without constructing a second in-memory inventory alongside the bin evidence.
#[derive(serde::Deserialize)]
pub(super) struct Document {
    schema_version: u32,
    workflow: String,
    pub bins: Vec<Value>,
}
pub(super) fn read_document(bytes: &[u8]) -> Result<Document> {
    if bytes.len() > MAX_BYTES {
        return Err(bad("report exceeds 256 MiB"));
    }
    let doc: Document = serde_json::from_slice(bytes).map_err(|e| bad(format!("invalid JSON: {e}")))?;
    if doc.schema_version != 2 || doc.workflow != "avid_managed_mxf" {
        return Err(bad("expected schema-2 avid_managed_mxf catalog"));
    }
    if doc.bins.is_empty() || doc.bins.len() > MAX_BINS {
        return Err(bad("expected 1–1024 bins"));
    }
    Ok(doc)
}

/// Defer composition diagnostics only when the caller will run the sequence converter.
pub fn plan_scoped(bytes: &[u8], mode: TransferMode) -> Result<Plan> {
    let doc = read_document(bytes)?;
    let bins = &doc.bins;
    let warnings =
        if mode == TransferMode::Metadata { vec!["Metadata-only import: media components remain unbound and clips are offline.".into()] } else { Vec::new() };
    let mut result = Plan { bins: Vec::new(), warnings };
    let mut count = 0usize;
    let mut mob_count = 0usize;
    let mut quantized_count = 0usize;
    for b in bins {
        if b.get("status").and_then(Value::as_str) != Some("parsed")
            || b.get("snapshot_stable") != Some(&Value::Bool(true))
            || !array(b, "diagnostics")?.is_empty()
        {
            return Err(bad("bin was not decoded completely from a stable snapshot"));
        }
        let name = bin_name(text(b, "path")?)?;
        let summary = b.get("summary").ok_or_else(|| bad("missing bin summary"))?;
        let uid = text(summary, "bin_uid")?.to_owned();
        let catalog = b.get("catalog").ok_or_else(|| bad("missing catalog"))?;
        let clips = array(catalog, "clips")?;
        count = count.checked_add(clips.len()).filter(|n| *n <= MAX_CLIPS).ok_or_else(|| bad("too many clips"))?;
        if catalog.get("summary").and_then(|s| s.get("master_count")).and_then(Value::as_u64) != Some(clips.len() as u64) {
            return Err(bad("incomplete master catalog"));
        }
        let mut out = BinPlan { name, uid, clips: Vec::new(), referenced: Vec::new(), subclips: Vec::new(), compositions: 0 };
        // Cross-check the derived catalog against independently exported decoded mobs.
        // Its own master_count cannot prove that no master was omitted during assembly.
        let mobs = array(b, "mobs")?;
        mob_count = mob_count.checked_add(mobs.len()).filter(|n| *n <= MAX_MOBS).ok_or_else(|| bad("too many decoded mobs"))?;
        let mut decoded_masters = BTreeMap::new();
        for m in mobs {
            if m.get("kind").and_then(Value::as_str) == Some("MasterMob") {
                let key = identity_key(m)?;
                membership(m)?;
                if decoded_masters.insert(key, m).is_some() {
                    return Err(bad("duplicate decoded master identity in bin"));
                }
            }
        }
        let rate_evidence = crate::avid_sequence::offline_rate_evidence(mobs)?;
        let mut sources = BTreeMap::new();
        for m in mobs.iter().filter(|m| m["kind"] == "SourceMob" && m["live"] == true) {
            if let Ok(id) = full_identity(&m["id"]) {
                // Duplicate live identities cannot authorize offline metadata.
                sources.entry(id).and_modify(|entry| *entry = None).or_insert(Some(m));
            }
        }
        let mut seen = BTreeSet::new();
        for c in clips {
            if c.get("kind").and_then(Value::as_str) != Some("MasterMob") {
                return Err(bad("catalog entry is not a master clip"));
            }
            let name = text(c, "name")?.to_owned();
            let key = identity_key(c)?;
            if !seen.insert(key.clone()) {
                return Err(bad("duplicate master identity in bin"));
            }
            let placed = membership(c)?;
            let decoded = decoded_masters.remove(&key).ok_or_else(|| bad("catalog master absent from decoded bin"))?;
            if membership(decoded)? != placed {
                return Err(bad("catalog master identity or membership disagrees with decoded bin"));
            }
            for field in ["name", "user", "length", "rate", "attributes"] {
                if c.get(field) != decoded.get(field) {
                    return Err(bad(format!("catalog master {field} disagrees with decoded bin")));
                }
            }
            let decoded_tracks = array(decoded, "tracks")?;
            let catalog_tracks = array(c, "tracks")?;
            if catalog_tracks.len() != decoded_tracks.len()
                || catalog_tracks.iter().zip(decoded_tracks).any(|(a, b)| ["index", "component", "properties"].iter().any(|k| a.get(k) != b.get(k)))
            {
                return Err(bad("catalog master tracks disagree with decoded bin"));
            }
            let user = c.get("user").and_then(Value::as_object).ok_or_else(|| bad("missing user metadata"))?;
            if user.contains_key(SOURCE_KEY) {
                return Err(bad("user metadata conflicts with reserved provenance key"));
            }
            for key in user.keys() {
                if key.len() > MAX_TEXT || key.contains('\0') {
                    return Err(bad("invalid metadata key"));
                }
            }
            let tracks = array(c, "tracks")?;
            if tracks.len() > 64 {
                return Err(bad("too many master tracks"));
            }
            for t in tracks {
                if t.get("index").and_then(Value::as_u64).filter(|n| *n > 0 && *n <= 65535).is_none() {
                    return Err(bad("invalid master track number"));
                }
                if !matches!(t.get("status").and_then(Value::as_str), Some("present" | "absent" | "partial" | "ambiguous" | "unresolved")) {
                    return Err(bad("invalid track availability"));
                }
            }
            let (duration, quantized) = duration(c)?;
            if quantized {
                quantized_count = quantized_count.saturating_add(1);
            }
            // Only placed masters are bin contents. A sequence transfer additionally needs the
            // masters its compositions reference, which Avid keeps in the file without placing
            // them; they are kept apart from the bin's own contents.
            if placed || mode == TransferMode::Sequences {
                let mut source_descriptors = Vec::new();
                let mut descriptor_ids = BTreeSet::new();
                for r in tracks.iter().filter_map(|t| t["references"].as_array()).flatten() {
                    if let Ok(id) = full_identity(&r["source_id"])
                        && let Some(Some(m)) = sources.get(&id)
                        && m["descriptor"].is_object()
                    {
                        if r.get("source_descriptor").is_some_and(|d| d != &m["descriptor"]) {
                            return Err(bad("source descriptor disagrees with decoded live source mob"));
                        }
                        if descriptor_ids.insert(id) {
                            source_descriptors.push(serde_json::json!({"id":m["id"],"descriptor":m["descriptor"]}));
                        }
                    }
                }
                let picture_rate_evidence = full_identity(&c["id"]).ok().and_then(|id| rate_evidence.get(&id)).cloned().flatten();
                let clip = ClipPlan { name, duration, evidence: c.clone(), source_descriptors, picture_rate_evidence };
                if placed { out.clips.push(clip) } else { out.referenced.push(clip) }
            } else {
                result.warnings.push(format!("{}: referenced master {name} was not imported as a bin item", out.name));
            }
        }
        if !out.referenced.is_empty() {
            result.warnings.push(format!(
                "{}: {} masters referenced by this bin's sequences, but not placed in it, were imported into a \"{REFERENCED_BIN}\" sub-bin so those sequences can resolve and relink their sources.",
                out.name,
                out.referenced.len()
            ));
        }
        if !decoded_masters.is_empty() {
            return Err(bad("catalog omitted decoded masters"));
        }
        // Masters this bin carries, whether placed or referenced: a subclip may only restrict one
        // of these, because its parent has to exist as an item for the range to mean anything.
        let available: BTreeMap<[u8; 32], Tick> =
            out.clips.iter().chain(out.referenced.iter()).filter_map(|c| Some((full_identity(&c.evidence["id"]).ok()?, c.duration))).collect();
        let mut clamped = 0usize;
        for m in mobs {
            if m.get("user_placed") == Some(&Value::Bool(true)) && m.get("kind").and_then(Value::as_str) != Some("MasterMob") {
                if m["kind"] == "CompositionMob" {
                    // A subclip is bin contents in its own right, not a composition to convert.
                    if m["usage_code"] == 2 {
                        match subclip(m, &available) {
                            Ok((plan, was_clamped)) => {
                                clamped += usize::from(was_clamped);
                                out.subclips.push(plan);
                                continue;
                            }
                            Err(e) => {
                                result.warnings.push(format!("{}: subclip {} was not imported: {e}", out.name, text(m, "name")?));
                                continue;
                            }
                        }
                    }
                    out.compositions += 1; // bounded by MAX_MOBS
                    if mode == TransferMode::Sequences {
                        continue;
                    }
                }
                result.warnings.push(format!("{}: unsupported bin item {} ({}) was not imported", out.name, text(m, "name")?, text(m, "kind")?));
            }
        }
        if out.subclips.len() > MAX_CLIPS {
            return Err(bad("too many subclips"));
        }
        if clamped > 0 {
            result.warnings.push(format!(
                "{}: {clamped} subclips record a range reaching past the end of the master they restrict; each was shortened to its master's extent and the original range is retained in source metadata.",
                out.name
            ));
        }
        result.bins.push(out);
    }
    if quantized_count > 0 {
        result.warnings.push(format!("For {quantized_count} clips, the recorded Avid rate gives a duration between FilmCraft ticks. Original counts and rates are retained in source metadata."));
    }
    Ok(result)
}
/// Validate depth and ID space before using the project's recursive bin helpers.
pub(crate) fn validate_project(project: &Project, parent: Option<BinId>) -> Result<()> {
    let mut stack = vec![(&project.root, 0usize)];
    let mut ids = BTreeSet::new();
    let mut found = parent.is_none();
    while let Some((bin, depth)) = stack.pop() {
        if depth >= 128 {
            return Err(bad("destination bin tree is too deep"));
        }
        if !ids.insert(bin.id.0) || bin.id.0 >= project.next_id {
            return Err(bad("inconsistent project bin IDs"));
        }
        if Some(bin.id) == parent {
            found = true;
        }
        for child in &bin.children {
            if let BinEntry::Bin(b) = child {
                stack.push((b, depth.saturating_add(1)));
            }
        }
    }
    if !found {
        return Err(bad("destination parent does not exist"));
    }
    if project.items.keys().any(|id| id.0 >= project.next_id || ids.contains(&id.0)) {
        return Err(bad("inconsistent project item IDs"));
    }
    Ok(())
}
/// The masters imported for one Avid bin, as `(placed contents, referenced sequence sources)`.
/// Media binding and sequence conversion must agree on the whole set, or a converted timeline
/// would hold clips that can never be relinked. They are returned apart because the two carry
/// different promises: the bin's own contents are what an import is accountable for, while a
/// referenced source that cannot be bound only costs its sequence clip a placeholder. Other
/// sub-bins are the user's and are never treated as sources.
pub fn imported_items(project: &Project, bin: BinId) -> Result<(Vec<filmcraft_project::ItemId>, Vec<filmcraft_project::ItemId>)> {
    let b = project.root.find_bin(bin).ok_or_else(|| bad("missing imported bin"))?;
    // Only masters. A subclip has no media of its own: it binds through the master it restricts,
    // so offering one to media binding would fail the item and, strictly, the whole import.
    let media = |id: &filmcraft_project::ItemId| project.item(*id).is_some_and(|i| i.as_media().is_some());
    let mut placed = Vec::new();
    let mut referenced = Vec::new();
    for child in &b.children {
        match child {
            BinEntry::Item(id) if media(id) => placed.push(*id),
            BinEntry::Item(_) => {}
            BinEntry::Bin(sub) if sub.name == REFERENCED_BIN => {
                referenced.extend(sub.children.iter().filter_map(|e| match e {
                    BinEntry::Item(id) if media(id) => Some(*id),
                    _ => None,
                }));
            }
            BinEntry::Bin(_) => {}
        }
    }
    Ok((placed, referenced))
}
fn allocate_name(names: &mut BTreeSet<String>, base: &str) -> Result<String> {
    if names.insert(base.into()) {
        return Ok(base.into());
    }
    for n in 2..=names.len().saturating_add(2) {
        let name = format!("{base}_{n}");
        if names.insert(name.clone()) {
            return Ok(name);
        }
    }
    Err(bad("cannot allocate bin name"))
}
#[derive(Debug, Serialize)]
pub struct PreviewBin {
    pub name: String,
    pub clips: usize,
    pub referenced: usize,
    pub subclips: usize,
}
/// Resolve all names together, accounting for other bins in this import without editing a project.
pub fn preview(project: &Project, plan: &Plan, parent: Option<BinId>) -> Result<Vec<PreviewBin>> {
    validate_project(project, parent)?;
    // One ID per bin and item, plus one for a referenced-sources sub-bin where it is needed.
    let required = plan
        .bins
        .iter()
        .try_fold(0u64, |n, b| {
            n.checked_add(1)?
                .checked_add(b.clips.len() as u64)?
                .checked_add(b.referenced.len() as u64)?
                .checked_add(b.subclips.len() as u64)?
                .checked_add(u64::from(!b.referenced.is_empty()))
        })
        .ok_or_else(|| bad("too many items"))?;
    project.next_id.checked_add(required).ok_or_else(|| bad("project ID space exhausted"))?;
    let bin = match parent {
        Some(id) => project.root.find_bin(id).ok_or_else(|| bad("destination parent does not exist"))?,
        None => &project.root,
    };
    let mut names: BTreeSet<String> = bin.children.iter().filter_map(|x| if let BinEntry::Bin(b) = x { Some(b.name.clone()) } else { None }).collect();
    plan.bins
        .iter()
        .map(|b| Ok(PreviewBin { name: allocate_name(&mut names, &b.name)?, clips: b.clips.len(), referenced: b.referenced.len(), subclips: b.subclips.len() }))
        .collect()
}
/// Add one offline master as an item of `bin`. `placed` records whether the Avid bin itself
/// listed it, so a referenced-only source is never reported as bin contents.
fn add_master(project: &mut Project, bin: BinId, bin_name: &str, bin_uid: &str, c: ClipPlan, placed: bool) -> Result<filmcraft_project::ItemId> {
    // Deliberately no guessed file path: multi-component sources need a media binding.
    let has_video = c
        .evidence
        .get("tracks")
        .and_then(Value::as_array)
        .is_some_and(|ts| ts.iter().any(|t| t.pointer("/component/media_kind").and_then(Value::as_u64) == Some(1)));
    let info = MediaInfo {
        name: c.name.clone(),
        kind: if has_video { MediaKind::Movie } else { MediaKind::AudioOnly },
        duration: c.duration,
        video: None,
        audio_streams: Vec::new(),
        container: "Avid metadata (unbound)".into(),
        start_timecode: None,
        file_size: None,
    };
    let mid = project.add_item(
        &c.name,
        if placed { Label::Lavender } else { Label::Tan },
        ItemKind::Media(MediaClip {
            media: MediaRef::File { path: String::new() },
            info,
            interpret: Interpretation::default(),
            mark_in: None,
            mark_out: None,
            markers: Vec::new(),
            offline: true,
            proxy: None,
            identity: None,
        }),
        Some(bin),
    );
    let item = project.item_mut(mid).ok_or_else(|| bad("new item missing"))?;
    if let Some(user) = c.evidence.get("user").and_then(Value::as_object) {
        for (k, v) in user {
            item.metadata.insert(k.clone(), v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()));
        }
    }
    let provenance = serde_json::json!({"version":1,"bin_name":bin_name,"bin_uid":bin_uid,"clip":c.evidence,"source_descriptors":c.source_descriptors,
        "picture_rate_evidence":c.picture_rate_evidence,"binding":"unbound","placed":placed});
    item.metadata.insert(SOURCE_KEY.into(), provenance.to_string());
    Ok(mid)
}
/// Apply a validated plan to a transaction-local project copy; no I/O or AVB dependency remains.
pub fn apply(project: &mut Project, plan: Plan, parent: Option<BinId>) -> Result<Vec<ImportedBin>> {
    let destinations = preview(project, &plan, parent)?;
    let mut imported = Vec::new();
    for (b, destination) in plan.bins.into_iter().zip(destinations) {
        let name = destination.name;
        let id = project.add_bin(&name, parent);
        let count = b.clips.len();
        let referenced = b.referenced.len();
        let subclips = b.subclips.len();
        // Identity to item, so a subclip can find the master it restricts. Placed and referenced
        // masters share one space: a subclip does not care which list its master came from.
        let mut masters = BTreeMap::new();
        for c in b.clips {
            let id_bytes = full_identity(&c.evidence["id"]).ok();
            let item = add_master(project, id, &b.name, &b.uid, c, true)?;
            if let Some(k) = id_bytes {
                masters.insert(k, item);
            }
        }
        if !b.referenced.is_empty() {
            // A separate sub-bin: these are sequence sources, not contents Avid showed in the bin.
            let sources = project.add_bin(REFERENCED_BIN, Some(id));
            for c in b.referenced {
                let id_bytes = full_identity(&c.evidence["id"]).ok();
                let item = add_master(project, sources, &b.name, &b.uid, c, false)?;
                if let Some(k) = id_bytes {
                    masters.insert(k, item);
                }
            }
        }
        // After their masters, so every parent already exists.
        for sub in b.subclips {
            let parent = *masters.get(&sub.parent).ok_or_else(|| bad("subclip master missing after import"))?;
            let item = project.add_item(
                &sub.name,
                Label::Iris,
                ItemKind::Subclip { parent, range: filmcraft_time::TimeRange::new(sub.start, sub.duration), restrict_trims: false },
                Some(id),
            );
            let item = project.item_mut(item).ok_or_else(|| bad("new subclip missing"))?;
            let provenance = serde_json::json!({"version":1,"bin_name":b.name,"bin_uid":b.uid,"clip":sub.evidence,
                "source_descriptors":[],"picture_rate_evidence":Value::Null,"binding":"through_parent","placed":true,"subclip":true});
            item.metadata.insert(SOURCE_KEY.into(), provenance.to_string());
        }
        imported.push(ImportedBin { bin: id, name, clips: count, referenced, subclips, compositions: b.compositions });
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::ItemKind;
    #[test]
    fn picture_rate_reconciliation_is_narrow_and_requires_actual_rate() {
        let actual = filmcraft_time::FrameRate::new(30000, 1001);
        for (n, d, expected) in [(2997, 100, true), (30000, 1001, true), (60000, 2002, true), (2998, 100, false), (30, 1, false), (0, 1, false), (24, 0, false)]
        {
            assert_eq!(picture_rate_matches(&serde_json::json!({"rate":{"numerator":n,"denominator":d}}), actual), expected);
        }
        assert!(!picture_rate_matches(&serde_json::json!({"rate":{"numerator":2997,"denominator":100}}), filmcraft_time::FrameRate::FPS_24));
    }
    use serde_json::json;
    fn fixture() -> Value {
        let mut v = json!({"schema_version":2,"workflow":"avid_managed_mxf","bins":[{
            "path":"C:\\Bins\\音声.avb","status":"parsed","snapshot_stable":true,"diagnostics":[],
            "summary":{"bin_uid":"synthetic"},"mobs":[],"catalog":{"summary":{"master_count":1},"clips":[{
                "name":"Source","kind":"MasterMob","user_placed":true,"id":{"raw":"01".repeat(32),"byte_order":"little"},
                "length":1,"rate":{"numerator":2997,"denominator":125},"user":{},"tracks":[]
            }]}
        }]});
        v["bins"][0]["mobs"] = json!([v["bins"][0]["catalog"]["clips"][0].clone()]);
        v
    }
    fn decode(v: &Value) -> Result<Plan> {
        plan(&serde_json::to_vec(v).unwrap())
    }
    /// Avid writes a subclip's range two ways and reading either alone is wrong somewhere. The
    /// numbers here are from a fixture whose in and out points a person set deliberately: a
    /// 334-frame master with a subclip over frames 47..83, which both forms must reproduce.
    fn subclip_fixture(attribute_form: bool) -> Value {
        let mut v = fixture();
        let master = v["bins"][0]["catalog"]["clips"][0].clone();
        v["bins"][0]["catalog"]["clips"][0]["length"] = json!(334);
        v["bins"][0]["mobs"][0]["length"] = json!(334);
        let rate = master["rate"].clone();
        // Attribute form: the component spans the whole master and the mob's length is the
        // master's, so only _START/_END describe the subclip. Component form: the reverse.
        let (component_start, mob_length, attributes) =
            if attribute_form { (0, 334, json!({"_START": 47, "_END": 83})) } else { (47, 36, json!({"_START": 0, "_END": 36})) };
        let sub = json!({"kind":"CompositionMob","name":"B.sub.01","live":true,"user_placed":true,
            "id":{"raw":"03".repeat(32),"byte_order":"little"},"length":mob_length,"rate":rate,"usage_code":2,
            "attributes":attributes,
            "tracks":[{"index":1,"component":{"class":"SCLP","media_kind":1,"length":mob_length,"rate":master["rate"],
                "source_id":master["id"],"source_track":1,"start":component_start}}]});
        v["bins"][0]["mobs"].as_array_mut().unwrap().push(sub);
        v
    }
    #[test]
    fn a_subclip_imports_the_range_avid_recorded_in_either_form() {
        for attribute_form in [true, false] {
            let v = subclip_fixture(attribute_form);
            let plan = plan_scoped(&serde_json::to_vec(&v).unwrap(), TransferMode::Sequences).unwrap();
            assert_eq!(plan.bins[0].subclips.len(), 1, "attribute_form={attribute_form}");
            let mut project = Project::default();
            let bins = apply(&mut project, plan, None).unwrap();
            assert_eq!((bins[0].clips, bins[0].subclips), (1, 1));
            let (parent, range) = project
                .items
                .values()
                .find_map(|i| match &i.kind {
                    ItemKind::Subclip { parent, range, .. } => Some((*parent, *range)),
                    _ => None,
                })
                .expect("a subclip item");
            // The parent is the master, and the range is 47..83 of its 334 frames, whichever way
            // Avid recorded it. Expected ticks are computed from the fixture's 2997/125 rate
            // directly, so this checks the range that was chosen rather than the conversion.
            let at = |frames: i128| (frames * 125 * i128::from(TICKS_PER_SECOND) / 2997) as i64;
            assert_eq!(range.start.0, at(47), "start, attribute_form={attribute_form}");
            assert_eq!(range.duration.0, at(36), "duration, attribute_form={attribute_form}");
            let master = project.item(parent).unwrap().as_media().unwrap();
            assert!(range.end() <= master.info.duration, "the range stays inside its master");
        }
    }
    #[test]
    fn a_subclip_reaching_past_its_master_is_bounded_by_it() {
        // Recorded ranges do run past a master's end, by a frame or by dozens, in a real project.
        // The item must not reach past its own media.
        let mut v = subclip_fixture(true);
        v["bins"][0]["mobs"][1]["attributes"]["_END"] = json!(400); // master is 334 frames
        let plan = plan_scoped(&serde_json::to_vec(&v).unwrap(), TransferMode::Sequences).unwrap();
        assert_eq!(plan.bins[0].subclips.len(), 1);
        assert!(plan.warnings.iter().any(|w| w.contains("reaching past the end of the master")), "{:?}", plan.warnings);
        let mut project = Project::default();
        apply(&mut project, plan, None).unwrap();
        let (parent, range) = project
            .items
            .values()
            .find_map(|i| match &i.kind {
                ItemKind::Subclip { parent, range, .. } => Some((*parent, *range)),
                _ => None,
            })
            .expect("a subclip item");
        let master = project.item(parent).unwrap().as_media().unwrap();
        assert_eq!(range.end(), master.info.duration, "shortened to exactly the master's extent");
        assert!(range.duration > Tick::ZERO);
        // Starting past the end is refused outright rather than clamped to nothing.
        let mut v = subclip_fixture(true);
        v["bins"][0]["mobs"][1]["attributes"]["_START"] = json!(500);
        v["bins"][0]["mobs"][1]["attributes"]["_END"] = json!(600);
        let plan = plan_scoped(&serde_json::to_vec(&v).unwrap(), TransferMode::Sequences).unwrap();
        assert!(plan.bins[0].subclips.is_empty());
        assert!(plan.warnings.iter().any(|w| w.contains("starts after the end of its master")), "{:?}", plan.warnings);
        // An offset with no usable end is refused, never stretched to the master's length.
        let mut v = subclip_fixture(true);
        v["bins"][0]["mobs"][1]["attributes"]["_END"] = json!(47);
        let plan = plan_scoped(&serde_json::to_vec(&v).unwrap(), TransferMode::Sequences).unwrap();
        assert!(plan.bins[0].subclips.is_empty());
        assert!(plan.warnings.iter().any(|w| w.contains("records a start without a usable end")), "{:?}", plan.warnings);
    }
    #[test]
    fn a_composition_that_is_not_a_plain_range_is_reported_rather_than_approximated() {
        // No master in this bin to restrict.
        let mut v = subclip_fixture(true);
        v["bins"][0]["mobs"][1]["tracks"][0]["component"]["source_id"]["raw"] = json!("09".repeat(32));
        let plan = plan_scoped(&serde_json::to_vec(&v).unwrap(), TransferMode::Sequences).unwrap();
        assert!(plan.bins[0].subclips.is_empty());
        assert!(plan.warnings.iter().any(|w| w.contains("its master is not in this bin")), "{:?}", plan.warnings);
        // An empty range is refused rather than imported as a zero-length item.
        let mut v = subclip_fixture(true);
        v["bins"][0]["mobs"][1]["attributes"]["_END"] = json!(47);
        v["bins"][0]["mobs"][1]["length"] = json!(0);
        let plan = plan_scoped(&serde_json::to_vec(&v).unwrap(), TransferMode::Sequences).unwrap();
        assert!(plan.bins[0].subclips.is_empty());
        assert!(plan.warnings.iter().any(|w| w.contains("subclip B.sub.01 was not imported")), "{:?}", plan.warnings);
        // Tracks disagreeing about which master, or where, is not one range.
        let mut v = subclip_fixture(true);
        let mut second = v["bins"][0]["mobs"][1]["tracks"][0].clone();
        second["index"] = json!(2);
        second["component"]["start"] = json!(9);
        v["bins"][0]["mobs"][1]["tracks"].as_array_mut().unwrap().push(second);
        let plan = plan_scoped(&serde_json::to_vec(&v).unwrap(), TransferMode::Sequences).unwrap();
        assert!(plan.bins[0].subclips.is_empty());
    }
    #[test]
    fn decimal_rate_is_preserved_with_explicit_subtick_quantization() {
        let v = fixture();
        let p = decode(&v).unwrap();
        assert_eq!(p.bins[0].clips[0].duration.0, (i128::from(TICKS_PER_SECOND) * 125 / 2997) as i64);
        assert_eq!(p.bins[0].clips[0].evidence["rate"], v["bins"][0]["catalog"]["clips"][0]["rate"]);
        assert!(p.warnings.iter().any(|w| w.contains("between FilmCraft ticks")));
        assert_eq!(p.bins[0].name, "音声");
    }
    #[test]
    fn invalid_catalogs_rejected_and_no_mutation_panics() {
        let cases = [
            ("/schema_version", json!(1)),
            ("/bins/0/snapshot_stable", json!(false)),
            ("/bins/0/diagnostics", json!(["incomplete"])),
            ("/bins/0/path", json!("😀")),
            ("/bins/0/catalog/summary/master_count", json!(2)),
            ("/bins/0/catalog/clips/0/length", json!(-1)),
            ("/bins/0/catalog/clips/0/length", json!(i64::MAX)),
            ("/bins/0/catalog/clips/0/rate/numerator", json!(0)),
            ("/bins/0/catalog/clips/0/rate/denominator", json!(0)),
            ("/bins/0/catalog/clips/0/id/raw", json!("urn:smpte:umid:01")),
            ("/bins/0/catalog/clips/0/id/byte_order", json!("middle")),
            ("/bins/0/catalog/clips/0/user", json!({SOURCE_KEY:"reserved"})),
            ("/bins/0/catalog/clips/0/user_placed", json!(null)),
            ("/bins/0/catalog/clips/0/tracks", json!([{"index":-1,"status":"present"}])),
        ];
        for (pointer, value) in cases {
            let mut v = fixture();
            *v.pointer_mut(pointer).unwrap() = value;
            assert!(decode(&v).is_err(), "{pointer}");
        }
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"][0]["length"] = json!(i64::MAX);
        v["bins"][0]["catalog"]["clips"][0]["rate"]["denominator"] = json!(i64::MAX);
        assert!(decode(&v).is_err());
        let bytes = serde_json::to_vec(&fixture()).unwrap();
        for end in 0..bytes.len() {
            assert!(std::panic::catch_unwind(|| plan(&bytes[..end])).unwrap().is_err());
        }
        for pos in 0..bytes.len() {
            let mut changed = bytes.clone();
            changed[pos] ^= 0xff;
            assert!(std::panic::catch_unwind(|| plan(&changed)).is_ok());
        }
        let deep = format!("{}0{}", "[".repeat(256), "]".repeat(256));
        assert!(std::panic::catch_unwind(|| plan(deep.as_bytes())).unwrap().is_err());
    }
    #[test]
    fn empty_repeated_bins_and_unsupported_items_are_explicit() {
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"] = json!([]);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(0);
        v["bins"][0]["mobs"] = json!([{"kind":"CompositionMob","name":"Sequence","user_placed":true}]);
        let b = v["bins"][0].clone();
        v["bins"].as_array_mut().unwrap().push(b);
        let p = decode(&v).unwrap();
        assert_eq!(p.warnings.len(), 3);
        let mut project = Project::new("Test");
        let out = apply(&mut project, p, None).unwrap();
        assert_eq!(out[0].name, "音声");
        assert_eq!(out[1].name, "音声_2");
        assert!(project.items.is_empty());
    }
    #[test]
    fn duplicate_master_identity_is_not_silently_deduplicated() {
        let mut v = fixture();
        let c = v["bins"][0]["catalog"]["clips"][0].clone();
        v["bins"][0]["catalog"]["clips"].as_array_mut().unwrap().push(c);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(2);
        assert!(decode(&v).unwrap_err().to_string().contains("duplicate"));
    }
    #[test]
    fn catalog_must_account_for_decoded_masters_and_membership() {
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"] = json!([]);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(0);
        assert!(decode(&v).is_err(), "a self-consistent count must not conceal an omitted master");
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"][0]["user_placed"] = json!(false);
        assert!(decode(&v).is_err(), "catalog membership must agree with the decoded bin");
    }
    #[test]
    fn mixed_byte_order_duplicate_master_is_rejected() {
        let mut v = fixture();
        let raw = "060a2b340101010501010f101300000002385d50221306a105dc1c1dd3d90a1b";
        let canonical = "060a2b340101010501010f1013000000505d38021322a10605dc1c1dd3d90a1b";
        v["bins"][0]["catalog"]["clips"][0]["id"]["raw"] = json!(raw);
        let mut second = v["bins"][0]["catalog"]["clips"][0].clone();
        second["id"] = json!({"raw":canonical,"byte_order":"big"});
        v["bins"][0]["catalog"]["clips"].as_array_mut().unwrap().push(second);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(2);
        v["bins"][0]["mobs"] = v["bins"][0]["catalog"]["clips"].clone();
        assert!(decode(&v).is_err(), "byte-order aliases must not import twice");
    }
    #[test]
    fn invalid_project_id_counter_is_rejected_before_changes() {
        for next in [0, u64::MAX] {
            let mut project = Project::new("Test");
            project.next_id = next;
            let before = project.clone();
            assert!(apply(&mut project, decode(&fixture()).unwrap(), None).is_err());
            assert_eq!(project, before);
        }
    }
}
