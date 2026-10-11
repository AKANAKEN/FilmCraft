# Avid bin import implementation plan

Agreed behavior: read an Avid bin once and transfer supported contents into ordinary FilmCraft project data. Do not embed the AVB, keep it open, synchronize it, or write to it. Referenced media stays in the selected Avid MediaFiles locations. Save changes in `.fcproj`.

Each imported AVB creates a new FilmCraft bin named after the AVB filename without its extension. Check sibling bins in the chosen parent: `Sound Effects`, then `Sound Effects_2`, `_3`, etc.; use the first available name. Never merge into or overwrite an existing bin automatically. An invalid parent is an error.

## Current increment — direct AVB import

The native reader supplies a desktop `File > Import Avid Bin…` workflow for one AVB and
optional managed MXF folders. `+ Add folder` adds another Browse field; extra rows can be removed.
Up to 32 roots, including separate volumes, are searched together recursively. Overlapping,
repeated or aliased root selections share canonical directory keys, so the same directory is not
scanned twice. Distinct copies with the same media identity remain ambiguous. The search shares
limits of 32 levels, 1,024 directories and 20,000 entries across all roots; limits or unreadable
paths are reported as incomplete. Selecting `avid media` includes its numbered children. Empty
folder lists import offline. Multiple-AVB selection remains follow-on UI work.
There is one import action: attempt clips, media and sequences together. Read and
review before committing. Changing a source path invalidates the preview. Cancel and failed preparation leave
project/history untouched; a successful import is one undoable edit.

Commands: `file.prepareAvidBin {path,parent?,mediaFolders?:[string],wait?}` starts the background job;
the original `mediaFolder` string is accepted for existing clients but cannot be combined with
`mediaFolders`. The array must contain at most 32 existing absolute folder paths.
`file.avidBinPreparation {job}` reports the prepared snapshot; `file.importAvidBin {job}` applies
that exact snapshot. The native default is best-effort import of all contents. Explicit restricted
modes remain available to research tests, not as UI choices. Native AVB preparation does not
invoke Python or read a JSON sidecar. It rereads and compares the source bytes after preparation,
including same-size changes, and rejects a changed destination project before apply.

Native coverage includes the tested metadata, managed source descriptors/locators and sequence
wrappers. An empty media-folder field retains supported offline components with their full IDs.
Folder search uses the existing bounded, cancellable identity matcher. Only a complete search
with one validated copy per component is applied automatically; other components remain offline
for Link Media. Names never substitute for identities. Failed media binding retains the clip
as offline metadata and reports its name/reason. A sequence uses the validated adapters first;
an unsupported effect with one input of equal duration/rate can be bypassed with a warning.
Other interpretable intervals receive labeled offline placeholders, preserving their positions
and lengths. When a composition has a known clock and duration but its tracks cannot be converted,
the importer retains it as a labeled placeholder sequence. Compositions without trustworthy timing
are reported individually without blocking other contents.
The report remains open after an import with errors, and diagnostics are saved with the project.
AMA binding and advanced rendering semantics remain outside this initial native playback path. Source AVBs, PMR/MDB files and media are never modified.

Large-library playback remains incomplete: the native Phase 6 all-content check retains all 1,953
masters, but only five can currently bind as offline sources; the other 1,948 retain metadata and
binding diagnostics. Most failures concern exact source timing; others concern missing source
evidence, track slots or descriptor ranges. Folder search currently follows successful descriptor
binding, so it cannot yet use MXF evidence to resolve those failed bindings. Indexing selected media
before binding is a follow-up; this result must not be presented as native Phase 6 playback support.

Automated native Phase 12 preparation links six components and produces the same media and
sequence data as the previously validated catalog workflow. Phases 9–12 also reconstruct offline
with the expected exact durations. Native project save/reopen and one-step undo/redo pass. The
user has confirmed the File-menu import and multiple-folder workflow in the test app.
Earlier sections below record the staged implementation and its historical test milestones.

## Before any GitHub push or submission

The contributor authorized fork creation, pushes and regular (non-draft) pull requests on
2026-10-11, after checks and review pass. Publishing remains conditional on the release tasks below.

- [x] Establish the GitHub workflow: use the contributor's `whatever-industries/filmcraft` fork,
  target `storytold/filmcraft:main`, and publish focused maintenance fixes before Avid importing.
  Use area-prefixed commits and descriptive branches; audit each final diff and run its gates.
  Open regular pull requests ready for review. Keep managed-media databasing and project
  modularity as separate proposals; document dependencies rather than closing #170 prematurely.
- [x] Do not add Codex authorship credit, generated-by text or Codex co-author trailers to commits,
  pull requests or code. Preserve required upstream license/provenance notices. Resolve contributor
  preferences under the repository's contributor rules before preparing a PR. The contributor
  chose `@whatever-industries` only; no personal-name entry is needed.
- [ ] Audit the complete intended diff again: correctness, hostile-input handling, exact timing,
  atomic failure/undo, save/reopen compatibility, media identity and partial-media behavior. Fix
  findings and rerun relevant regressions; run the required repository gates before committing.
  Record remaining failures honestly rather than claiming a green build.
- [ ] Structure changes into cohesive modules and reviewable commits/PRs with clear dependencies.
  Remove accidental edits, duplication and temporary diagnostics; keep private media, reports,
  generated projects and local test launchers out of submissions.
- [ ] Review documentation against the final code: command contracts, supported/unsupported cases,
  schema compatibility, design decisions, test evidence and reproduction steps. Replace stale
  status claims and give each PR a focused problem/behavior/validation description.
- [ ] Finish localizing the direct AVB dialog's plain English labels. The catalog gates cover
  menu labels and strings marked for translation; they do not establish full dialog localization.

## A — Data-transfer foundation (this increment)

The foundation has passed a [targeted code audit](avid-code-audit.md), with five reproduced
reader/import validation defects fixed and sample acceptance repeated. The full checkpoint gate
passed on 2026-10-11 after the GPU, caption-picker, translation and UI-test timing findings were
resolved. This is not yet a submitted importer. The separate managed-audio binding command below
provides a limited playback path.

A second [managed-import audit](avid-code-audit.md#second-audit--managed-playback-sequences-and-combined-import)
reproduced and fixed four further consistency/completeness defects after sequence conversion and
combined import were added. Valid corpus output is unchanged; its final validation and remaining
boundaries are recorded separately from the original foundation results.

Implement a pure-Rust catalog adapter and headless commands using the existing standalone reader's schema-2 JSON. Python decoders remain outside the FilmCraft product. This development input is explicitly an Avid catalog, not a direct AVB reader.

Acceptance:

- Import empty bins and master-clip names, user metadata, identities, component timing and source-location evidence into new bins with the agreed suffix rule.
- Preserve missing A1 / present A2 evidence without renumbering channels or inferring playback availability.
- Validate format/version, bounds, timings, parent IDs and incomplete/unstable bin reads before changing the project.
- Make the entire import one undo/redo action. Errors leave project, selection and history unchanged.
- Save/open the FilmCraft project with no AVB or catalog access required afterwards.
- Transfer metadata as offline placeholders initially. Do not guess a single file for a multi-file master or claim that identity matching proves codec playback.
- Exercise synthetic hostile inputs, real Phase 4/5 metadata and Phase 6 counts. Keep private AVBs/media/screenshots outside the checkout.

## B — Managed media binding and playback

Existing OP-Atom support reads each file independently. `MxfSource` can combine compatible sound tracks inside the opened file, but does not locate companion files. The MXF README explicitly lists external essence across OP-Atom files as unsupported. This stage therefore adds identity-based companion resolution and a logical source above the existing per-file decoder; it does not replace that decoder. An Avid master and its channel grouping must determine how the separate essence files are exposed together.

Design and implement one logical source backed by multiple managed MXF components. Preserve original track labels, per-component rates/lengths/offsets and missing components. Do not collapse an absent A1 into present A2. Reuse FilmCraft's MXF decoder where supported; distinguish found files from decodable media. Check mono, stereo split across files, partial stereo and picture-plus-audio using existing samples. Test placement, seeking, playback, save/reopen and relinking. Phase 10 adds required stereo-pair semantics: the master still references two mono MXFs, while its paired channels feed one stereo timeline track. Keep source component, source channel and timeline track identities separate. This may require a source-model/schema change; do not disguise masters as ordinary editing sequences merely to avoid that work.

Implemented first slice of B (development commands; no import UI yet):

- Native schema 16 adds `MediaRef::AudioComponents`. Each logical channel stores its Avid track number, full file-package UMID, optional path, file stream/channel and exact source range. Original rational timing and source-track evidence remain in imported provenance.
- `media.bindAvidAudio` verifies the actual file-package identity and supported single-track OP-Atom profile before binding. It currently accepts direct managed mono PCM components with a common sample rate, retaining every logical slot from A1. Missing slots render silence; available A2 never shifts into A1. Ambiguous references, picture and wrappers are rejected before an edit.
- Phase 9's source channels place as two mono clips. Phase 10's `ST:A1A2,` master grouping places as one stereo clip, using `AudioChannelMap`. This verifies native placement, not conversion of the saved Avid sequence.
- Native MXF decoding now handles bounded opaque header packets and legacy AAF sound/timecode identifiers. Phase 9's three mono MXFs match external FFmpeg PCM at four seek positions per file.
- Component bindings survive native project save/reopen and undo. `media.relinkAudioComponent` validates the expected full identity before replacing one path. Status reports each component separately. Replacement files with matching names but different package IDs are rejected.
- Multi-file proxies, generic Make Offline, collect/consolidate and reference-based interchange export are not supported yet. Destructive/single-file operations reject component references; OTIO/comp export reports missing/gap warnings. Picture-plus-audio and AMA binding remain follow-on work.

Automated coverage includes missing A1, distinct channel signals, bounded requests, wrong-identity relink rejection, relink undo, native schema roundtrip, actual-media seeks, stereo timeline mixing and save/reopen. A user listening test remains useful for device playback and monitor interaction.

## C — Direct AVB import workflow

`File > Import Avid Bin…`: choose one or more AVBs; optionally choose managed-media roots; preview imported/unsupported items and unresolved/ambiguous locations; commit the import. Metadata-only import is allowed. Scanning must be cancellable, bounded and outside the UI thread. Register command/automation controls and test the UI. Keep AVB metadata authoritative; use PMR/MDB as media-location/identity evidence. A native decoder or a separately agreed delivery approach is needed before shipping the direct-file workflow; the product's current repository rules require pure Rust. Existing decoder-derived research is not claimed to be a clean-room native implementation.

### Reuse Link Media for missing components

FilmCraft already provides `File > Link Media…`, candidate matching, folder search and automatic
relinking. Extend this workflow rather than creating a separate Avid repair dialog. The current
single-file relink command explicitly rejects component sources; dedicated audio/picture component
commands already validate full file-package identity before replacement. Component UI integration
is now implemented (validation and limits below).

- [x] Represent missing rows with a stable item plus component key (picture track or logical audio
  slot), not only an item ID. Group V1/A1/A2 under their logical clip; show previous path and status.
- [x] Let the user choose media roots or locate one component, then search for companions and other
  missing components. Reuse bounded/cancellable search infrastructure. Resolve managed MXF by full
  package identity and supported profile; filenames/timecode alone must not authorize replacement.
- [x] Preview exact matches, absent files, unreadable files and ambiguous duplicate identities.
  Preserve channel slots and expose per-component diagnostics; verify paths again before applying.
- [x] Apply accepted matches in one undoable edit. Allow leaving unresolved components offline and
  reconnecting later, without changing clip/sequence identity, source ranges or timeline placement.
  Distinguish genuinely missing media from unsupported decoding/sequence semantics.
- [x] Separate search results from applied links in the dialog: show Match found / Choose a copy,
  explain that Search leaves media offline, and show the number of components the Link action
  will reconnect. Keep skipped components out of subsequent batches.
- [x] Retain validated offline component structure during import for the observed managed mono
  PCM and progressive 1080 picture profiles. Exact adapter evidence permits the tested 29.97
  source without its MXF. Broader profiles still require media or metadata-only import.
- [x] Cover moved folders, changed mount roots, partial stereo, wrong-ID same-name files, ambiguous
  matches, cancellation, save/reopen and undo. Ordinary loose-file/AMA relinking remains a separate
  matching policy from managed MXF identities.

The user supplied a relink-dialog screenshot as a behavior reference. No third-party screenshot,
artwork or UI asset is copied into the repository or product.

## D — Broader format coverage and native migration

MDB first for memory reduction, then AVB, subject to upstream provenance rules and parity tests. Preserve decoder diagnostics and supported unknown data. Extend only with explicit coverage: subclips/groups, sequences/effects, markers, Windows captures, AMA source remapping and unresolved timecode semantics. Keep write research separate on disposable files. No routine new baselines or rebuilds are needed now.

The standalone reader's MDB-first replacement priority remains separate from the product's
direct-file import dependency. The latter now has a native AVB container/membership foundation
(`file.inspectAvidBin`); neither Python helper has been replaced by a complete native decoder.

### Native AVB stages for the direct-file workflow

- [x] Bounded container/object index and observed ABIN membership; preserve opaque payloads and
  unknown classes. Compare all membership references with an external decoder oracle.
- [ ] Decode CMPO roles, persistent identities and track references; typed attributes and current
  bin metadata; preserve unknown extensions with explicit diagnostics.
- [ ] Decode source descriptors/locators, keeping managed MXF and AMA evidence separate.
- [ ] Decode the supported editorial graph (cuts, filler, timecode, validated automatic adapters
  and channel combiners); bound cycles/depth and report unsupported structures.
- [ ] Establish semantic parity across Phases 1–12 before wiring cancellable import preparation
  and the File-menu preview/commit workflow. No Phase 13 expansion is required for this stage.

Structural inspection currently matches 25 private AVBs / 110,429 objects, including Phase 6's
108,338 objects, 6,934 membership entries and 1,953 user-placed entries. These are structural
counts, not native semantic import acceptance. All real files are little-endian; big-endian
support is synthetic-test-only. See the interchange README for provenance and coverage.

### Deferred until after the initial pull request: Phase 13

The user supplied a private, bloated feature-film bin collection as Phase 13, including very
complex sequences in `01 CURRENT CUT`. On 2026-10-11 the user explicitly requested that this
corpus be logged for later work, not investigated now. No Phase 13 files have been inspected.

- [ ] After the initial PR, inventory this corpus and establish read-only source hashes.
- [ ] Measure large-project/bin decode time and peak memory separately from media scanning.
- [ ] Use `01 CURRENT CUT` to identify complex sequence structures and unsupported cases;
  preserve explicit omissions and distinguish structural parsing from faithful conversion.
- [ ] Propose follow-up coverage and tests from those findings, rather than expanding the first PR.

Phase 13 is not an initial-PR acceptance requirement. Keep all original bins, media, identifying
metadata and generated reports private; use Phases 1–12 and synthetic cases for current work.

## D1 — Sequence/timeline research and conversion

Track this explicitly as a separate capability. Phase 8 now provides one Avid-authored 24 fps stringout: four Phase 7 AMA masters, 505 frames, source-ins zero, with gaps and A1/A2 distinctions. Phase 9 adds a 240-frame managed-MXF audio stringout at 24 fps, with two unchanged Phase 5 masters, three verified MXFs, and an empty V1 track. Use these existing baselines first; no further manual sample is needed to start. Later compare an AAF export of the same sequence and extend it with trims/nonzero source-ins rather than repeating baselines. Master-clip acceptance alone cannot establish timeline support.

First sequence iteration: cuts, source ranges, track layout, gaps and timing. The user explicitly accepts deferring effects and speed changes; list omitted treatment and never claim timing fidelity for a dropped retime. Phase 8 contains automatic spatial-adapter wrappers around video even without user-applied effects: carry their child references structurally before translating cuts. Phase 10 additionally supplies master channel grouping, timeline `_TRACK_FORMAT` and an audio-channel-combiner wrapper with two inputs. Channel combiners are required routing, not optional creative effects. The research helper now carries bounded child references and track properties, retaining unsupported semantics flags. Auxiliary capture-mask/timecode tracks must not be mistaken for picture retimes. Identify tracks by media kind plus index (V1, A1 and TC1 coexist), and validate primary sequence timecode separately from auxiliary rates. Phase 9's empty V1 must remain empty, not imply picture essence.

Document composition identity, track kind/number, sequence start/rate, source in/out, filler, mixed-rate time domains, audio offsets, subclip/group references, transition overlap/handles, markers, nested sequences, effects and retimes. Set explicit conversion coverage and failure/omission reporting before implementing each subset. Check frame positions and durations against Avid-visible values and exported AAF; compare selected rendered frames/audio when playback is supported. Unknown structures must never disappear silently.

Candidate comparison tools (README/documentation reviewed 2026-10-10; implementation code not consulted for this product adapter):

- [otio-avb-adapter](https://github.com/markreidvfx/otio-avb-adapter): Apache-2.0; uses pyavb. Its documented read matrix covers clips, multiple video/audio tracks, gaps, markers, nesting and transitions, but excludes effects, speed effects and writing. Evaluate as a pinned external AVB-to-OTIO comparison tool against FilmCraft's existing OTIO importer. This is a candidate research route, not yet a validated shipping path. Because it shares pyavb, agreement is not independent evidence of AVB decoding correctness; use Avid/AAF observations too.
- [avid-media-composer-mcp](https://github.com/leancoderkavy/avid-media-composer-mcp): MIT; documented saved-bin inspection and sequence/source queries may aid analysis. AVB inspection also uses pyavb. Live native qualification is Windows/build-specific and Mac qualification is deferred; it is not evidence of a ready Mac editing or full-fidelity conversion bridge.

Keep these tools outside the pure-Rust product. Review licenses, pin exact versions and record observed losses before executing comparisons. Follow AGENTS.md's clean-room rule; prior decoder-derived research must remain clearly attributed and cannot be relabeled as clean-room.

### Color transfer

Plan item and sequence colors. Preserve raw `_COLOR_R`, `_COLOR_G`, `_COLOR_B` separately from FilmCraft display labels. Phase 8's green sequence and Phase 9's pale-purple sequence supply known observations; verify scaling and palette semantics before mapping. Prefer exact RGB when supported; otherwise document any nearest-label mapping or retain evidence with a warning. Absence of color attributes means unset/default, not RGB black. Exact visual color matching is optional in the first iteration. The current adapter retains master attributes as evidence and assigns its default placeholder label; it does not yet translate colors or sequences.

## D2 — Separate managed-media catalog proposal

Read-only PMR/MDB resolution is groundwork for, not completion of, persistent native background indexing and reconnection. Reuse existing relink, watch and transcode facilities; add measured persistence, collision handling, restart/rebuild and shared-storage rules as a separately scoped issue. See [issue breakdown](avid-issue-breakdown.md) for proposed acceptance criteria and PR boundaries. Prepare changes locally now; update GitHub when submitting the work.

## E — Modular projects (future independent PR)

After usable Avid import, implement reversible single-file-to-modular conversion, stable identities, module ownership and transactional multi-file saves. Independent-bin collaboration is later work. No modular-storage dependency is required for importing an Avid bin.

## Development commands and verification

The initial commands accept an already decoded schema-2 research report, not an AVB path:

```json
{"method":"engine.execute","params":{"command":"file.previewAvidCatalog","params":{"path":"/absolute/path/catalog.json","parent":null}}}
{"method":"engine.execute","params":{"command":"file.importAvidCatalog","params":{"path":"/absolute/path/catalog.json","parent":null}}}
```

`parent` may be a valid bin ID or omitted for the root. With `mode` omitted (or `"metadata"`),
responses include `bins`, `warnings` and `mediaBinding: "unbound"`; placeholders have no playable
streams. Import records one undo step. These development commands have no File-menu entry and
do not start Python. The optional combined modes below use the same component/sequence validators
as the separate development commands.

### Combined import preflight and transaction

Both catalog commands accept `mode: "metadata" | "media" | "sequences"`:

- `metadata` is the backward-compatible default: transfer bin/master metadata and report omissions.
- `media` also binds imported masters to verified managed MXF components or supported offline descriptors.
- `sequences` additionally converts placed compositions into their corresponding new bins.
  Bins with no compositions, including empty bins, are valid.

```json
{"method":"engine.execute","params":{"command":"file.previewAvidCatalog","params":{"path":"/absolute/path/catalog.json","mode":"sequences"}}}
{"method":"engine.execute","params":{"command":"file.importAvidCatalog","params":{"path":"/absolute/path/catalog.json","mode":"sequences"}}}
```

Each operation reads one bounded report snapshot and prepares the requested stages on a disposable
native project copy. Preview does not edit the session, history, selection or media cache. Preview
IDs/names are prospective, not reserved; import revalidates the current report, project and media.
An invalid or unsupported requested stage fails the whole operation, including previously prepared
bins. Nothing is silently retried as metadata-only. Users can explicitly choose metadata mode when
media binding or sequence support is unavailable. Existing partial-channel handling is preserved;
found files must still pass identity/profile checks, and missing A1 never becomes A2.

Successful import commits everything in one undo step, opens the first converted sequence at its
start and retains the normal sibling suffix rules. Results include `bound` per-master diagnostics
and `sequences` with destination bin, name, exact tick duration and frame rate. `bins[].compositions`
counts source compositions, including ones omitted in metadata/media mode. Media work is sequential
in batches of at most 64 masters; batches do not create extra undo snapshots.

This is engine workflow groundwork, not the shipping import dialog. The commands remain synchronous;
cancellable background preparation and a reviewable UI are required before exposing large imports
through the File menu. Direct AVB decoding, media-root selection/remapping and AMA binding remain
separate dependencies. The combined mode does not expand codec or sequence coverage.

Combined-import validation: 562 engine tests and 106 format/interchange tests pass. Four new
synthetic regressions cover preview immutability, one report read per operation, one-step undo/redo,
native save/reopen, repeated-bin scoping, invalid modes, late binding/sequence failures, empty bins,
missing A1/present A2, and 65-master batching. The opt-in Phase 9–12 comparison produces native
projects identical to the existing separate-command pipeline. Metadata corpus acceptance remains
unchanged for Phases 4/5 and Phase 6's 1,953 masters (1,761 fully represented, 192 partial).
Targeted all-target Clippy, formatting, whitespace, layering and WASM checks pass. Logs remain
local under `target/avid-combined-*`. No new manual Avid test is needed for this transaction change;
the previously recorded full-workspace GPU precision failure remains outside these targeted gates.

Validation commands:

```sh
cargo test -p filmcraft-interchange -p filmcraft-engine avid_catalog --lib
FILMCRAFT_AVID_REPORTS=/absolute/path/to/acceptance-reports \
  cargo test -p filmcraft-engine avid_catalog_local_corpus --lib -- --ignored --nocapture
```

The opt-in corpus test uses existing `phase4-04.json`, `phase5-02.json` and `phase6-full.json`,
verifies all transferred clip evidence, metadata, partial-channel distinctions, source report
bytes and native project encode/decode. Private samples are not committed. Test reports remain
local build output. Normal import never writes a source report, AVB or media file.


Additional opt-in routing checks use the transported Phase 8–10 reports:

```sh
FILMCRAFT_AVID_PHASE8_REPORT=/absolute/path/phase8-transport.json \
FILMCRAFT_AVID_PHASE9_REPORT=/absolute/path/phase9-transport.json \
FILMCRAFT_AVID_PHASE10_REPORT=/absolute/path/phase10-transport.json \
  cargo test -p filmcraft-engine avid_catalog_local_sequences --lib -- --ignored --nocapture
```

These verify master transfer/save/reopen, stereo grouping/property retention and explicit sequence
omissions. They are not timeline-conversion or playback acceptance. Source and report snapshots
remain local; the committed tests contain only synthetic data and environment-variable hooks.


Managed-audio development commands (item IDs come from the imported native project):

```json
{"method":"engine.execute","params":{"command":"media.bindAvidAudio","params":{"items":[1,2]}}}
{"method":"engine.execute","params":{"command":"media.relinkAudioComponent","params":{"item":1,"channel":0,"path":"/absolute/path/A1.mxf"}}}
```

Binding accepts 1–64 selected masters and commits one atomic undo step. Component relink uses a
zero-based logical channel index; it does not guess a match from a filename. Neither command writes
source media. Direct AVB decoding and automatic root selection remain separate work.

```sh
cargo test -p filmcraft-engine component_audio --lib
FILMCRAFT_AVID_AUDIO_ROOT=/absolute/path/to/Phase9/avid/media/002 \
  cargo test -p filmcraft-engine avid_audio_local_mxf_decoding_matches_oracle --lib -- --ignored
FILMCRAFT_AVID_PHASE9_REPORT=/absolute/path/phase9-transport.json \
FILMCRAFT_AVID_PHASE10_REPORT=/absolute/path/phase10-transport.json \
FILMCRAFT_AVID_PLAYBACK_OUTPUT=/absolute/path/playback.fcproj \
  cargo test -p filmcraft-engine component_audio_local_binding_placement_and_reopen --lib -- --ignored
```

The optional output is a disposable native five-second stereo playback project. It references the
original mono MXFs and contains no embedded AVB or catalog dependency.

### Local playback verification — 2026-10-10

The generated five-second stereo project opens in the desktop app, with the correct timeline clip
and duration. A short device-playback smoke check recorded 176 callbacks, zero underruns and zero
missing frames. This verifies the playback path, not human-perceived sound/channel fidelity; the
listening procedure is in local `target/Avid Audio Test.txt` alongside the launcher and project.
The source AVB/database hashes still match the existing reports; full hashes of the three audio
MXFs are retained in local `target/avid-audio-source-hashes.json`.

All 549 engine tests in the broader run passed, as did 139 format/MXF/project/interchange tests.
An additional binding-atomicity test and the focused component suite pass. Workspace Clippy,
layering, attribution and WASM checks are recorded in `target/avid-audio-*.log`. The earlier full
workspace release CI stopped at the unchanged GPU extract-effect precision test, so there is no
claim of a green full release gate and no commit or PR has been made.


### Phase 11 audible replacement

The user identified Phase 9/10's five-second audio as silence. Those tests retain structural,
identity and silence-decoding value, but do not establish real-signal decoding fidelity.
Phase 11 supplies one 531-frame/24 fps managed master backed by two mono 48 kHz, 16-bit PCM
MXFs. Both channels are non-silent and sample-identical. The new opt-in
`component_audio_local_audible_pcm_and_timeline` test compares every source sample with the
external FFmpeg oracle, requires non-silent signal, verifies combined channels, timeline mixing,
seeking and native save/reopen. Use `FILMCRAFT_AVID_PHASE11_REPORT` and optional
`FILMCRAFT_AVID_PLAYBACK_OUTPUT` to run it; private data stays outside the checkout.

Each MXF contains 1,062,001 samples; the AVB's 531-frame master range represents 1,062,000.
The test keeps the AVB duration (22.125 seconds), without extending it by the extra file sample.
The master has two mono channels, with no stereo-pair attribute. The generated listening project
therefore places two mono clips, explicitly panning A1 left and A2 right for isolated listening.
This is test routing, not inferred Avid pan metadata or sequence conversion. Identical source
signals cannot reveal a channel swap by ear; distinct synthetic channels still validate routing.
Local evidence: `target/phase11-playback-test-final.log`, `target/phase11-signal.json` and
`target/phase11-source-hashes.json`. The launcher now opens `Avid Phase 11 Playback.fcproj`;
the original five-second project is retained.


### Phase 12 — managed picture/audio, trims and mixed rates

The user confirmed Phase 11 audible playback works. Phase 12 is the next structural acceptance
sample: one 24 fps sequence, 152 frames (00:00:06:08), starting at 01:00:00:00; three masters,
six MXFs with verified identities, 12 live MDB joins and no missing media. The screenshot agrees
with the decoded duration, source rates and A1/A2/A3 layout. All ten supplied-file hashes are
unchanged after inspection. Private evidence and the repeatable assertions are outside this
checkout in the standalone reader's `reports/phase12-*.json` and `research/analyze_phase12.py`.

Positions below are half-open frame ranges relative to sequence start at 24 fps:

| Track | Sequence range | Source / interpretation |
| --- | --- | --- |
| V1 | [0, 88) | 24 fps picture, source in 21 |
| A3 | [0, 88) | Same master's A1/A2 combined as a stereo pair, both source in 21 |
| A1, A2 | [81, 119) | Separate sound-effect master, dual mono, source in 87 |
| V1 | [88, 106) | Explicit 18-frame picture gap |
| V1 | [106, 152) | Picture-only 30000/1001 fps source, source in zero; 46 output frames |

The separate audio overlaps the first clip by seven frames and continues across the gap and
into the second picture. This captures nonzero source-ins and overlap without another sample.
Both pictures are 1920x1080 DNx-family MXF; the first master has one picture and two mono PCM
files, with `ST:A1A2,` grouping. The second master has no sound track.

The second picture is wrapped by a spatial adapter and a `SPED` motion adapter
(`EFF_TIMEWARP.MOTION_CTL`, `_MIXMATCH_MOTIONADAPTER=2`). The saved speed ratio is 1001/1250,
matching 24 divided by the independently probed MXF rate 30000/1001. Its source component spans
58 source-rate frames while the outer component spans 46 timeline frames. Keep those frame
domains separate. The helper exposes the AVB rate as decimal 29.97 (2997/100), so exact MXF-rate
evidence must remain distinct; a global decimal-to-rational replacement would be unjustified.
Exact source-frame selection, interpolation and phase are not validated yet.

Acceptance additions:

- Bind the first master's three MXFs as one picture-plus-audio source with stable channels.
- Preserve trim offsets, gaps, overlap, source channel mappings and the stereo combiner.
- Treat the mixed-rate motion adapter as timing-critical. Until validated, report the affected
  segment explicitly as unsupported; do not silently flatten it when optional effects are omitted.
- Keep auxiliary timecode capture masks and edgecode separate from picture motion adaptation.
- Preserve the green sequence's raw color attributes; display-color mapping remains optional.

This sample is sufficient for the next implementation work. No replacement baseline or new
manual test is requested now. This analysis does not claim FilmCraft video playback or sequence
conversion has been implemented.


### Picture-plus-audio binding implementation

Schema 17 generalizes the source reference to `MediaRef::Components { picture, channels }`.
The deserializer accepts the earlier `AudioComponents` variant with no picture field, preserving
schema-16 playback projects. `media.bindAvidMedia` adds one identified picture component to the
existing managed-audio path; `media.bindAvidAudio` retains its audio-only scope. Single-source
`SEQU` wrappers with only zero-length filler are accepted, with bounded nesting and rejection of
incomplete/unsupported branches. More complex source graphs remain unsupported.

The picture component stores its original track number, full file-package identity, optional path,
source start and length. The logical source delegates picture and audio decoding to independently
verified MXFs, retaining exact time offsets and stable audio slots. Missing picture displays an
explicit slate while surviving audio remains playable. `media.relinkPictureComponent` verifies
identity and format before changing the path; relink/undo invalidate the media cache. Per-component
status and missing-media scans include picture. Proxies, collect/consolidate, generic Make Offline
and reference-based interchange export remain explicitly unsupported for component sources.

The Phase 12 first master binds one 24 fps picture file and two mono PCM files, exposed as its
saved stereo pairing. Automated evidence compares all 484,000 audio samples in each channel and
five out-of-order 1920x1080 YUV picture frames against FFmpeg (maximum difference observed: one
8-bit code value). Native placement, timeline mixing, save/reopen and picture relink/undo are
covered. Private-file tests use `FILMCRAFT_AVID_PHASE12_REPORT` and optional
`FILMCRAFT_AVID_PLAYBACK_OUTPUT`; the product does not execute FFmpeg or Python.

The generated `target/Avid Phase 12 Playback.fcproj` is a constructed full-length source playback
test (242 frames at 24 fps), not reconstruction of the six-second Avid sequence. The separate
29.97 fps picture master remains unbound until explicit AVB/MXF rate reconciliation is supported.
Mixed-rate motion-adapter frame selection and sequence conversion remain pending; no silent
adapter removal or approximate conversion is claimed. The user confirmed this picture/audio
playback test works. Their Project panel screenshot exposed a general sequence-display omission:
Audio Info now reads the sequence's output sample rate and layout, alongside its video settings.
Audio Codec remains source-only; Audio Usage counts placements in other sequences, not contained
audio tracks. Existing projects receive the display correction without reimporting or rebinding.


Validation evidence for this increment: 551 engine tests plus 44 format/interchange tests passed
in the broad run; the added source-wrapper regression also passes. Final focused component and
private Phase 12 oracle tests pass, including loading the schema-16 Phase 11 project and picture
relink/undo. Workspace Clippy, final test Clippy, formatting, layering, asset attribution and WASM
checks pass. The desktop smoke test displays picture and stereo meters with no Link Media prompt;
46 frames shown, zero drops, 179 audio callbacks, zero underruns/missing frames in the short run.
Logs and screenshot are local `target/phase12-*` outputs. The earlier unrelated GPU precision
failure still prevents claiming a green full release CI; no commit or PR is made.

### First native sequence conversion

`file.previewAvidSequences {path, bin}` and `file.importAvidSequences {path, bin}` consume the
schema-2 report after catalog transfer and managed-media binding. The explicit destination bin
scopes source identity matching: repeated imports elsewhere cannot select the wrong master.
Preview reports supported sequences and rejected compositions separately. Import refuses the
entire batch if any composition is rejected, and otherwise creates native sequences in one
undoable transaction in that bin. No AVB/report access is needed after saving the native project.

Current coverage is deliberately HD 1080p/24: direct source clips, exact source in-points,
sequential filler, primary start timecode, sparse picture/audio track numbers, mono source-channel
selection and two-input stereo combiners. Combiner input order is preserved. A1/A2 source slots
are not renumbered when media is missing. Source ranges, component rates/kinds/lengths, complete
structures and identities are checked before placement. Empty V1 remains empty. Same-rate
picture placement has a synthetic regression; actual managed-audio sequences are validated below.

All original composition attributes, including raw RGB color and auxiliary tracks, remain in
`filmcraft:avid-sequence`. Display-color mapping, markers, clip links, Avid gain/pan automation,
effects and nested sequences are not translated. Sequence output uses the explicit FilmCraft
default of 48 kHz stereo; mono tracks use native center pan. Track mute/solo are transferred when
present. These limitations are reported by preview/import. Unknown effects and retimes reject the
composition; they are not silently flattened. Nonempty unknown auxiliary tracks, truncated trees,
nonrepresentable timing, unbound/ambiguous masters and trailing filler that would shorten a native
sequence are rejected. Traversal, allocations, rates and new-ID counts are bounded.

Local corpus results:

- Phase 9: three placed clips, 240 frames; A2's second-half gap and empty V1 preserved.
- Phase 10: two placed clips, 240 frames; A3 is stereo with channels [0,1], while mono A1 starts
  at frame 120; empty A2/V1 preserved.
- Phase 11: two mono clips, 531 frames; start timecode 01:00:00:00, decoded source vs native
  timeline mix checked at four positions including the measured signal peak; save/reopen passes.
- Phase 12: rejected without edits at frame 106 (spatial adapter containing the mixed-rate motion
  adapter). Both frame rates are known: output 24, source MXF 30000/1001. Rejection is missing
  adapter conversion/validation, not inability to identify rates. A basic explicitly approximate
  frame-rate conversion is a follow-on option; exact Avid selection/interpolation remains unverified.

The new local `target/Avid Phase 11 Converted Sequence.fcproj` is converted from the saved Avid
sequence, unlike the earlier manually constructed source-playback test. Its separate launcher
leaves the previous playback projects intact. Native MDB/AVB decoding and the File-menu import
workflow remain separate milestones. This is still a development catalog bridge.

Validation for this increment: 556 engine tests and 105 format/interchange unit/integration tests
passed, plus the opt-in Phase 9–12 corpus check. Clippy for the changed crates/all targets,
formatting, diff whitespace checks, layering and WASM (44 crates) pass. The rebuilt desktop opens
the converted Phase 11 sequence with two mono waveforms and active meters. A short playback run
recorded 1,431 audio callbacks with zero underruns/missing frames (361 frames shown, six display
frames dropped during concurrent validation). This is a smoke check, not a playback benchmark.
Logs/screenshots remain local under `target/avid-sequence-*` and `target/avid-converted-sequence-*`.
The earlier unrelated GPU full-workspace CI failure remains unresolved; no commit or PR was made.

### Phase 12 adapter mapping (supersedes the initial rejection above)

The user confirmed the converted Phase 11 project displays 22:03, two mono tracks and empty V1.
The next increment maps the observed automatic Avid rate adapter to FilmCraft Frame Sampling,
with 100% playback speed. Its `1001/1250` ratio is checked against output/source edit rates
(24 divided by 30000/1001); it is not copied into the native playback-speed field. The original
46 output frames and source offset survive. This implements FilmCraft's equivalent behavior;
it does not claim exact Avid interpolation or frame-selection parity.

Picture binding opens the exact identified single-track MXF before interpreting its frame counts.
The observed Avid decimal 29.97 representation is accepted against verified MXF 30000/1001,
only for picture-only rate reconciliation. Other mismatches still fail. Raw counts/rates remain
unchanged in source evidence and the exact rate decision is recorded separately. A single-source
spatial adapter maps to Scale to Frame Size, with a warning that Avid crop/reformat treatment may
differ. Conversion warnings are returned by preview/import and saved with composition evidence.

Accepted motion adapters must have the automatic-adapter marker, matching positive rational
ratio, zero phase/offset, one complete picture source, and a source span covering the output
rounded up to one source frame. Creative/variable speed effects, unsupported modes, malformed
padding, unknown branches, truncated graphs and inconsistent source bounds remain rejected.
Synthetic tests include nonzero source-ins, all selected frames over a 48-frame segment, invalid
ratios/rates/ranges/modes and native save/reopen. Native Motion auto-points are resolved using
verified source geometry before the converted sequence is stored.

Phase 12 now imports five clips across V1 and A1/A2/A3, retaining duration 152 frames (6:08),
primary timecode 01:00:00:00, the 18-frame picture gap and overlapping trimmed audio. Timeline
frames 106/107/118/151 select MXF frames 0/1/14/56 with native frame sampling; full-resolution YUV
comparisons against external FFmpeg differ by at most one 8-bit code value. Rendered gap frames
88/100/105 are black, and the dual-mono mix during the gap matches the source at its trimmed
position. The tests reopen the native project before rendering. No new source fixture was needed.

Local output is `target/Avid Phase 12 Converted Sequence.fcproj`, launched by
`target/Run Avid Phase 12 Sequence Test.command`; the older source-playback project and converted
Phase 11 project remain separate. These are still development catalog commands, not direct AVB
File-menu import or a claim that general Avid effect conversion is complete.

Validation for the adapter increment: 558 engine tests and 106 format/interchange tests pass,
plus both opt-in corpus/playback checks. Targeted all-target Clippy, formatting, whitespace,
layering and WASM (44 crates) checks pass; the desktop build succeeds. An isolated desktop smoke
test displays the final mixed-rate shot at timeline frame 118 and duration 6:08. Playback reports
152 shown frames, zero drops, 594 audio callbacks and zero underruns/missing frames. This short
run is not a performance benchmark. All ten original Phase 12 files match the saved SHA-256
manifest. Logs/screenshots are local `target/avid-rate-*` artifacts. The previously recorded
unrelated full-workspace GPU precision failure remains open; these targeted results do not
establish a green release gate. No commit, push or GitHub submission has been made.

User playback confirmation: the Phase 12 conversion works in the desktop test. The supplied
screenshots show the final clip at 100% speed, Frame Sampling, and duration 1:22 within the 6:08
sequence. Its yellow render bar is expected: native preview classification requires matching
source/sequence frame rates for a no-bar clip. This source is 30000/1001 fps in a 24 fps sequence;
the yellow tooltip means unrendered but expected to play in real time. It does not indicate an
unintended speed change. One second of source time still occupies one second of timeline time.

Follow-up before claiming motion-adapter fidelity:

- [ ] Establish the saved render-mode mapping using Avid-visible settings and controlled outputs.
  Phase 12 retains `_SFX_RENDER: 2` and `_SFX_IGNORE_RENDER_SETTING: 1`; their numeric meaning has
  not been verified. The ratio and `mc_mode: 0` alone do not establish interpolation behavior.
  Avid documents configurable adapter rendering; see its
  [motion-adapter instructions](https://kb.avid.com/pkb/articles/en_US/how_to/How-to-adjust-motion-adapter-parameters).
- [ ] Compare an Avid-rendered reference of the existing final shot frame by frame, including
  cadence, phase and blend weights. Current FFmpeg comparisons verify source decoding and native
  frame selection, not equality with Avid's rendered sequence.
- [ ] Address the native interpolation gap before offering other mixed-rate conversion choices:
  `filmcraft_render::interpolation_blend` currently returns no blend at absolute speed 1.0,
  regardless of differing source/sequence rates. Verify Frame Blending and Optical Flow for
  mixed-rate 100% playback with suitable timing/boundary regressions before mapping to them.

Keep 100% speed for this validated real-time adapter. Frame Sampling remains an explicitly
approximate first-pass policy, not a proven match to the saved Avid render mode or a universal
quality preference. Yellow remains the existing native preview-cost indicator.


### Component Link Media implementation

The AVB-derived full file-package identity retained in each native binding is authoritative.
The user chooses a search folder; a unique verified MXF match resolves without filename or
manual identity choices. Picture and each logical audio slot appear separately in Link Media.
An explicit Link applies the selected match and other unique matches in one undo step. Multiple
files carrying one expected identity are copies to choose between; this is not a claim that the
AVB lacks a clip identity. No arbitrary filename, path order or truncated-ID tie-break is used.
Leaving A1 offline does not hide A2. Changing paths preserves native clip/sequence IDs, timing,
channel layout and retained source evidence. Regular file/AMA matching retains its existing policy.

Commands (all usable through automation):

- `media.searchComponents {folder, targets?: [{item,component}], wait?: false}` starts a read-only
  background job. Component keys are `{kind:"picture"}` or `{kind:"audio",channel:0}` (zero-based
  logical channel slot). Defaults to all missing components, at most 4,096 per search.
- `media.componentSearchResult {job}` returns progress and per-component candidates, identity
  verification, `unique`/`ambiguous`/`unreadable`/`absent` status, and diagnostics. An incomplete
  search uses `unresolved` instead of asserting absence. `jobs.cancel {job}` cancels search.
- `media.applyComponentSearch {job, choice?: {item,component,path}, relinkOthers?: false}` applies
  unique results, or an explicitly chosen copy. Results expire after a project edit or another
  search. Each file is reopened and checked against the current identity, profile, source range
  and saved format before any project mutation. A failure aborts the whole batch.
- `media.relinkComponents {bindings:[{item,component,path}]}` validates an explicit batch. Existing
  picture/audio component commands delegate to the same validator; Locate uses that path.

Search visits at most 20,000 entries, 1,024 directories and 32 nested levels; symbolic links are
not followed. Each MXF metadata scan permits at most 64 MiB of reads and 100,000 read calls.
Results retain at most 16 copies per identity, 8,192 candidates and 128 diagnostics (with an
omitted count). Known OP-Atom companion-track references and bounded opaque Avid packet notices are retained
without marking identity coverage incomplete. Other parser warnings, read errors, skipped links
and exhausted limits make search
incomplete and disable automatic batch matching; an explicitly chosen verified candidate can
still be linked. Only the latest search result is retained. Search does not modify media or
PMR/MDB/AVB files. Folder searching is desktop-only; explicit component relinking uses Services.

Offline-aware preparation is now implemented for the validated profiles described below.
A dedicated efficient header-only index can later replace the bounded native MXF demux walk;
large-folder benchmarking and broader format coverage remain follow-up work.

### Offline rate interpretation: research and decision

Research reviewed 2026-10-10. A decoded decimal rate is not sufficient evidence that Avid performs
edit arithmetic using that literal decimal. Avid's public
[MediaCentral rate documentation](https://developer.avid.com/ctms/api/aa/resources/keyframes.html)
explicitly identifies 29.97 with 30000/1001, 23.976 with 24000/1001, and 59.94 with 60000/1001.
This documents Avid's rate convention, not the binary encoding of AVB fields. Preserve the original
decoded value separately from the interpreted playback rate; a research report's 2997/100
normalization does not establish that AVB stores that rational pair.

The existing Phase 12 report supplies independent evidence without opening media: the final
picture's automatic SPED adapter has output rate 24/1 and speed_ratio [1001,1250]. Under the
validated output/source ratio interpretation, 24 / (1001/1250) is exactly 30000/1001, matching
the previously verified MXF. The child source sequence has 58 source frames and the adapter
occupies 46 timeline frames. Retain those separate lengths; do not reconstruct the timeline
length by rounding a converted source length. The offline import path uses this evidence; it does not establish that the render mode matches Avid.

Relevant public findings:

- Avid's [Media Composer 2020.12.1 fixes](https://resources.avid.com/SupportFiles/attach/README_Avid_Editor_v20.12.1.pdf),
  page 4, describe frame shifts after promoting mixed-rate material and changing speed
  (MCCET-3378), and a one-frame sync shift when adding/removing Timewarp with adaptive
  deinterlacing (MCCET-3578). These are specific historical defects, not proof of a general
  incorrect NTSC clock or a defect in the supplied sample.
- Avid's [AirSpeed 1.5.4 release notes](https://resources.avid.com/SupportFiles/attach/README_AirSpeed_Multi_Stream_v1_5.4.pdf),
  page 11, record a sample/frame rounding bug fixed in 1.5.3 (UDevC00145754). It concerned
  errors at the end of 29.97/59.94 recordings in AirSpeed, a separate product.
- The first-hand [OpenTimelineIO issue 468](https://github.com/AcademySoftwareFoundation/OpenTimelineIO/issues/468)
  describes a one-frame downstream shift during mixed-rate Premiere XML conversion. The
  discussion corrects the initial arithmetic explanation and emphasizes retaining explicit
  timeline extents instead of deriving them from source duration. It is an interchange
  caution, not an Avid bug report.
- In a [Shutter Encoder report and developer response](https://www.reddit.com/r/shutterencoder/comments/qydnc6/),
  a literal 2997/100 output rate triggered an MXF rejection; the developer's workaround sent
  30000/1001. This is direct evidence that passing a nominal decimal through as an exact rate
  can break interchange, independent of visible drift.

Decision implemented by the offline importer:

- Do not reject Phase 12 merely because its decoded rate reads 29.97. Validate the automatic
  adapter, master identity, source track and rate agreement, and use its exact rational evidence.
- Preserve raw rate evidence and its interpretation basis. Cross-check identified MXF when
  available; a genuine contradictory exact rate must not be silently overwritten.
- Keep supported offline clip identities, sequence positions and durations for later relinking.
  Missing media prevents playback, not reconstruction when the AVB supplies sufficient timing.
- Keep sequence edit units, source edit units, timecode numbering, and render cadence separate.
  Matching rates does not establish Avid's frame selection, blending, or phase behavior.
- Do not globally snap arbitrary nearby decimals to standard rates. Establish a bounded,
  documented Avid nominal-rate policy for cases without corroborating media/adapter evidence;
  that broader policy remains unvalidated.

Regression coverage: Phase 12 import with all media unavailable retains the final clip
at frame 106 for 46 frames and the complete 152-frame sequence; later identity-checked relink
does not move edits. Tests also cover conflicting/malformed adapter evidence, long-duration
exact-rate math, and nonzero source in-points. A synthetic 4,096-cut regression now checks every
edit boundary and selected source frames across 155,680 timeline frames, plus save/reopen and
undo/redo. It does not validate Avid's render cadence. A literal 29.97 clock differs from
30000/1001 by one part per million (about 3.6 ms per hour); short playback cannot validate it.
Frame-by-frame Avid-render comparison remains the separate cadence test already listed above.

Validation: 573 engine tests, the eight component relink cases including Phase 12, media UI
regressions, targeted Clippy, desktop build, layering and WASM pass. The desktop test verified
one-action relink of six components, one-step undo/redo and native save/reopen. Ten original
Phase 12 SHA-256 hashes remain unchanged. Details and the still-open workspace GPU gate are
recorded in `avid-code-audit.md`. The separate local launcher
`target/Run Avid Component Relink Test.command` opens a disposable project whose unavailable
paths are intentional; selecting the original Phase 12 media folder exercises identity matching.


### Offline component import implementation

`file.previewAvidCatalog` and `file.importAvidCatalog` accept `missingMedia: "offline" | "error"`.
The default is `offline`; metadata-only mode remains unchanged. Supported missing components
retain full identities, logical channel slots, source ranges and available paths. Only a file-not-found
result authorizes fallback for a recorded location. Permission errors, existing corrupt files and
identity/profile mismatches abort preparation without changing the project. `error` retains the
previous binding requirements, including the established absent-channel/verified-companion case.

For missing audio, live unique SourceMob descriptors must establish mono PCM format, integral
sample rate, sample width and sufficient duration. A fully missing logical source needs evidence
for every channel. Older reports with an absent slot can still use a decoded companion's rate.
For missing picture, the currently supported descriptor profile is progressive 1920x1080 8-bit
4:2:2 with the observed display aspects. 24 fps is exact; the tested nominal 29.97 requires matching
constant automatic-adapter evidence for 30000/1001. Conflicting clocks disable that fallback.
No generic nearby-rate rounding or unsupported effect interpretation is introduced.

Schema 18 adds `PictureComponent.format_pending`, default false for existing verified bindings.
Offline picture geometry/rate enables sequence reconstruction; decoder-specific format remains
pending. Playback stays on an offline slate until identity-checked relink. Relink validates source
range, raster, exact rate, pixel aspect and sample layout before replacing provisional codec/color
information and clearing the flag. Existing verified bindings retain full format equality checks.
Reappearing paths alone do not bypass pending picture verification; Link Media still lists them.

Results expose per-master `offlineComponents` and `mediaBinding: "offline_components"` when
applicable. Rate evidence and the original decoded values remain in project provenance. Relinking
changes neither timeline placement nor duration. Direct AVB import and its File-menu workflow are
still separate work; this implementation consumes the existing decoded catalog contract.


Offline-import validation: 577 engine unit tests, 32 interchange unit tests and 17 format unit
tests pass; 14 private/opt-in engine cases are excluded from the broad run. The separate catalog
corpus run passes all 13 cases with Phases 9–12 enabled, including offline Phase 12 and online
versus staged conversion. Phase 12 was rerun after adding rejection checks for incompatible
replacement rate, aspect and raster. Clippy for engine/interchange/format all targets, layering,
WASM (44 crates), formatting and desktop build pass. The existing unrelated GPU workspace gate
remains open; this is not a release-gate claim.

An isolated desktop session opened `target/Avid Offline Import Test.fcproj` with six unavailable
components. Searching the original Phase 12 media folder and pressing Link restored all six in
one action; undo restored six missing entries, redo cleared them, and native save/reopen reported
zero missing components. Inspected screenshots show the offline slate, verified match preview,
and restored 6:08 timeline. The disposable user launcher is
`target/Run Avid Offline Import Test.command`. No original file was moved or written; all ten
Phase 12 SHA-256 hashes match. No commit, push or GitHub submission has been performed.
