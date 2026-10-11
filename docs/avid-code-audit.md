# Avid reader and import foundation audit — 2026-10-10

This document retains the original foundation audit below and a subsequent audit of managed
playback, sequence conversion and the combined import at the end. Validation counts apply to
their stated increment, not to the entire application or all Avid formats.

Scope: the new FilmCraft catalog adapter, command boundary, transactions and tests; the local
standalone Rust reader's PMR/MXF parsing, identity matching, catalog assembly, subprocess boundary,
snapshot/output handling and report comparison; and the recent AVB structural-transport extension.
This is a targeted source review and regression exercise, not certification of all FilmCraft code
or third-party decoders. No playback or sequence-conversion support is claimed.

## Reproduced findings and fixes

Each case below failed in a regression test before its fix and passes afterwards.

| Finding | Consequence before the fix | Correction |
| --- | --- | --- |
| Catalog completeness checked only against its own count | Removing a master and decreasing the count silently omitted a bin item | Cross-check canonical identities and placement against decoded masters; reject missing, extra or membership-inconsistent entries before editing |
| Duplicate detection used raw bytes plus byte-order label | Two encodings of the same full identity could create duplicate items | Normalize the material-number fields for equality; retain original evidence unchanged |
| Short-ID collision checks excluded a previously unseen query | A source reference could match a short ID already associated with a different full identity | Include the queried full identity in collision evidence; return ambiguous |
| Leaf traversal discarded truncated sequence nodes | A supported branch plus an omitted branch could be reported present | Retain empty, unsupported or depth-limited sequences as unresolved evidence and bound traversal depth |
| Helper stderr attached before envelope validation | A malformed JSON array accompanied by stderr caused a Rust panic | Validate the object envelope before adding diagnostics; return a per-file error |

The first two fixes live in `crates/interchange/src/avid_catalog.rs`. The remaining three are
in the separate research reader's `identity.rs`, `catalog.rs` and `bridge.rs`; that reader is not
bundled into FilmCraft. The regression corpus and private source media remain outside this checkout.

## Validation

- Rust reader: 25 regression tests pass, including the three reproduced reader failures.
- FilmCraft adapter/commands: 12 synthetic tests pass, including omission and byte-order cases,
  invalid-input atomicity, undo/redo, name collisions, and saved-project independence.
- Python transport: three synthetic tests pass for wrappers, routing properties, shared references,
  cycle detection and traversal limits.
- Targeted Clippy with warnings denied passes for the reader and FilmCraft engine/interchange.
- Fresh reader acceptance: 21 scan runs across Phases 1–7 and the example corpus pass,
  including Python/FFprobe comparisons, sparse/full sampling independence and v1/v2 report
  compatibility. All 68 tracked source hashes are unchanged. Phase 6 remains 1,953 masters,
  6,934 live MDB matches, 3,043 verified MXF identities and 1,761 complete / 192 partial clips.
- Two opt-in FilmCraft corpus tests pass against the existing reports: Phase 4/5 metadata,
  all Phase 6 clip evidence and missing-channel distinctions, Phase 8–10 master metadata and
  explicit sequence omissions, stereo grouping, and native project save/reopen.
- Formatting and whitespace checks pass. The earlier full workspace release gate completed with an unchanged GPU test failure
  (`gpu_effects_match_cpu_exactly`, extract effect, maximum relative difference 1.33e-4). It
  started before these fixes and is not validation of the final patch. A complete gate on the final state remains required before committing or submitting.

Local evidence: the reader's `reports/audit-before.log`, `reports/audit-after.log`,
`reports/audit-acceptance.log` and `reports/rust-acceptance-vmmml4es/acceptance.json`;
FilmCraft's `target/avid-audit-before.log`, `target/avid-audit-after.log`,
`target/avid-audit-corpus.log` and `target/avid-audit-clippy.log`. These are uncommitted local output.

## Boundaries before the next implementation

Keep metadata transfer separate from media binding: filesystem presence does not establish codec
playback or Avid online status. Preserve structural audio combiners even when creative effects are
omitted. Multi-file media, relinking and sequence placement need their own acceptance checks.

The Python helper remains a trusted, pinned local subprocess. Its time/output limits are not a
memory sandbox; native MDB migration remains relevant to the large sample's memory cost.
Unsupported metadata and timecode paths remain explicit limitations. Private sample success does
not establish coverage for unseen AVB versions, effects or malformed third-party decoder inputs.

## Second audit — managed playback, sequences and combined import

Scope: the accumulated native component model/format migration, media opening/binding/relinking,
sequence transfer (including the automatic rate adapter), combined preview/commit command, relevant
MXF changes and failure/roundtrip tests. The standalone research reader and third-party decoders
were not re-audited in this increment. No GitHub submission or release certification is implied.

Four validation defects were reproduced with failing tests before their fixes:

| Finding | Consequence | Correction |
| --- | --- | --- |
| Binding trusted derived references without comparing them with the bin source clip | A valid MXF could be bound despite conflicting source identity, track, offset or duration evidence | Compare canonical full IDs, source tracks and exact audio timing; compare picture offsets/counts before verified-rate reconciliation |
| Source-wrapper traversal filtered zero-length filler before validating it | Truncation/unsupported flags and incompatible kind/rate on discarded padding went unnoticed | Validate every child before removing permitted zero-length filler |
| Composition and primary-timecode completeness checks were incomplete | A flagged incomplete composition or track could still become an apparently complete native sequence | Reject incomplete composition/track/timecode evidence before planning; preserve the deliberate exception for untranslated auxiliary timecode/edgecode semantics |
| Catalog cross-checking covered master identity and membership only | Derived names, metadata, timing or source tracks could disagree with decoded bin objects | Check names, user metadata, lengths/rates, attributes and ordered source-track structure against the decoded masters; derived location/status fields remain separate |

Tests cover both preview/import failure paths through the existing transaction tests. Binding
fixtures now include the actual source-reference fields required by the transport contract;
tests do not weaken validation to accommodate incomplete synthetic data. Source files are read only.

The corpus caught an overbroad intermediate rejection of known auxiliary timecode/edgecode objects.
That check was narrowed: their untranslated semantics stay in evidence, while primary timecode
must be supported and completeness markers remain enforced. Phase 9–12 comparisons pass after
this correction. This is why synthetic tests alone are insufficient for accepting a parser change.

Final validation for this increment:

- 566 engine tests pass; 12 private/environment-dependent tests are opt-in.
- 136 format/interchange/MXF unit and integration tests pass, including existing parser mutations.
- Phase 4/5 metadata and Phase 6's 1,953 masters still transfer and save/reopen, preserving
  1,761 fully represented / 192 partial clips.
- Combined import equals the previously validated separate-command result for Phases 9–12.
  One undo step, bin scoping, empty bins, partial channels and failure atomicity remain covered.
- Phase 12 retains 152 frames and five clips. Selected native frames still differ from external
  FFmpeg source decoding by at most one 8-bit YUV code value; gap/trimmed-audio checks pass.
- All ten Phase 12 original-file SHA-256 hashes match the saved manifest.
- Targeted all-target Clippy with warnings denied, formatting, whitespace, layering and WASM
  (44 crates) checks pass. These do not replace the required full-workspace release gates.

Local evidence: `target/avid-audit2-before.log`, `avid-audit2-catalog-before.log`,
`avid-audit2-engine.log`, `avid-audit2-format-mxf.log`, `avid-audit2-corpus.log`,
`avid-audit2-metadata-corpus.log`, `avid-audit2-playback.log`, `avid-audit2-source-hashes.txt`,
`avid-audit2-clippy.log`, `avid-audit2-layers.log`, `avid-audit2-wasm.log` and
`avid-audit2-build.log` (all under local `target/`, not submitted).

Remaining known boundaries: native direct AVB import, cancellable UI preparation, component-aware
Link Media and offline import are pending. Exact Avid frame-selection/interpolation equivalence
is unverified; FilmCraft's 100%-speed blending limitation remains deferred as agreed. General
effects, broader rates/formats and unseen malformed inputs are not covered by these results.
The previously recorded unrelated GPU precision failure still prevents claiming a green full
release gate; it was not rerun or reclassified as passing during this targeted audit.

## Component relink increment

Managed media now uses retained full AVB-derived file-package identities in the existing Link
Media workflow. The new engine module separates bounded read-only search from atomic path
replacement; existing component commands share its validator. Search results carry stable item
and component keys. Stale project results, duplicate targets, wrong identities, changed files,
incompatible formats and invalid ranges fail before project mutation. Duplicate media identities
are surfaced as multiple copies to choose between. Incomplete/cancelled scans cannot supply an
automatic batch; explicit verified choices remain possible.

The first Phase 12 run caught an overbroad diagnostic policy: expected OP-Atom companion-track
references and the already validated opaque Avid header packets had incorrectly disabled batch
matching. These exact native parser notices are now retained without marking identity coverage
incomplete. Unknown diagnostics, truncation, sync loss and missing local essence remain blocking.
A regression tests this distinction; the real sample resolves all six components uniquely.

Validation:

- 573 engine unit tests passed, 13 opt-in tests ignored. All eight component relink tests also
  passed with the local Phase 12 case enabled: renamed Unicode paths, wrong-ID same-name files,
  duplicate copies, partial A1/A2, stale results, atomic rollback, undo, native roundtrip, hostile
  parameters, cancellation/read budgets, symbolic-link cycles and diagnostic classification.
- Nine media UI tests passed; the three Link Media tests were rerun after the final layout and
  duplicate-choice assertions. Leaving A1 offline keeps A2 available; duplicate results require
  an explicit copy choice. Ordinary file relinking and constrained-viewport tests still pass.
- Targeted all-target Clippy, formatting, whitespace checks, layering, WASM (44 crates) and desktop
  build passed. This does not supersede the previously recorded unrelated GPU workspace failure.
- An isolated desktop instance opened a disposable Phase 12 project with six unavailable paths,
  searched the original folder, linked all six in one action, undid them together, redid, saved and
  reopened with no missing media. Screenshots were inspected; long rows retain V1/A1/A2 and status,
  with full names/paths on hover. The six-component relink reproduces the original native project
  exactly in the engine corpus test. All ten original Phase 12 SHA-256 hashes are unchanged.

Logs and disposable projects/screenshots remain under ignored `target/component-*` and
`target/Avid Component Relink Test.fcproj`. No production media/project was changed, and no
commit, push or GitHub submission was made. Offline-aware first import, native AVB decoding,
large-folder performance work and comprehensive effect conversion remain separate follow-ups.


## Offline first import and exact adapter clocks

The importer now retains supported managed components when initial media lookup returns
NotFound. Permission failures and present-but-invalid media are not converted into offline
successes. Unique live SourceMob descriptors authorize PCM structure; inactive history does not
create ambiguity, while duplicate live identities cannot authorize fallback. Missing slots beside
a verified companion preserve the previous partial-audio behavior.

The exact-clock parser is shared between offline evidence extraction and sequence conversion,
with bounded traversal and checked rational arithmetic. The observed 24 fps / (1001/1250) adapter
establishes 30000/1001 without its MXF. Original decoded rates and interpretation basis remain in
provenance. Conflicting clocks disable fallback. Timeline extents are not reconstructed from
rounded source durations. Unsupported profiles remain explicit errors.

Schema 18 persists pending picture format separately from its established geometry/rate. Pending
picture stays on a slate, including if the former path reappears. Relink validates full identity,
range, geometry, rate, pixel aspect and pixel layout, then hydrates decoder-specific metadata in
one atomic edit. A mismatch leaves all requested bindings and undo history unchanged.

Validation: 577 engine, 32 interchange and 17 format unit tests pass; the separate 13-case catalog
run enables local Phases 9–12. Offline Phase 12 reproduces the online 152-frame sequence and,
after relink, identical native item contents. Additional rate/aspect/raster rejection checks pass.
Targeted all-target Clippy, desktop build, layers, WASM (44 crates), formatting and whitespace
checks pass. The earlier unrelated full-workspace GPU precision failure still prevents a blanket
release-gate claim.

Native desktop verification: six missing components shown, folder search finds verified identities,
one Link restores six, undo/redo toggles six/zero, and save/reopen reports zero. Screenshots were
inspected. Disposable outputs are `target/avid-offline-*` and `target/Avid Offline Import Test.fcproj`;
original Phase 12 hashes remain unchanged. No commit or external publication was performed.

Remaining: direct AVB/File-menu import, wider offline descriptor/rate coverage, repeated-cut stress
coverage, and Avid-render cadence comparison. Current Frame Sampling remains an approximation of
Avid's rendering, despite the exact source clock and preserved edit boundaries.

## Relink clarity and status consistency audit

Reviewed the search/apply boundary, complete-versus-incomplete scan handling, explicit duplicate
choices, skipped component slots, stale project snapshots, atomic replacement, and offline picture
status. This is a focused follow-up, not a complete release audit of every accumulated change.

Fixed one correctness finding: `media.status` opened pending picture directly and could report it
online when its old path reappeared, although playback correctly required an identity-checked relink.
The Phase 12 regression reproduced this before the fix. Status and playback now share the bound
picture check; status exposes `formatPending`, and only successful relink clears it. The shared
check also rejects a changed saved picture format. Playback keeps that component on a slate with
a diagnostic, preserving separately available audio. Candidate validation can still open the
underlying picture to verify and hydrate its pending format before the atomic edit.

The dialog now labels search evidence as Match found, Choose a copy, Copy selected, Not found,
Cannot link or Search incomplete. It explains that Search does not reconnect media, and the Link
button counts the selected component plus eligible unique matches. Toggling companion linking
updates this count; incomplete scans require explicit selection. Stable automation IDs expose
row status and the next action. Ordinary file relinking retains its existing matching policy.

Validation:

- 577 engine tests and nine media UI tests pass; 14 environment-dependent engine cases remain
  opt-in. The separate Phase 12 offline/relink regression passes, including pending status before
  relink and online status afterward. A synthetic pending-status assertion also covers this reason
  without private media. UI coverage checks search without mutation, batch-count toggling, leaving
  A1 offline, duplicate selection, and explicit selection after incomplete scans.
- In an isolated desktop instance, all six Phase 12 rows show Match found and the action reads
  Link 6 files. The actual screenshot was inspected. Linking clears all six missing components;
  undo restores six, redo clears six, and save/reopen retains zero missing components.
- Targeted engine/UI all-target Clippy with warnings denied, desktop build, formatting, whitespace
  and layer checks pass. All ten original Phase 12 hashes remain unchanged. Full release gates
  remain outstanding, including the previously recorded unrelated GPU precision failure.

Local evidence is under ignored `target/relink-clarity-*` and `target/relink-audit-*`, including the
before/after regression logs and desktop screenshot. No commit, push or GitHub submission occurred.

## Stress coverage, catalog memory and native AVB framing (2026-10-11)

Added a synthetic 4,096-cut mixed-rate sequence with varied filler, durations and nonzero source
in-points. It covers 155,680 frames at 24 fps (over 108 minutes), checking every edit boundary
and first/middle/last sampled source frames against exact integer arithmetic. Native save/reopen
and undo/redo preserve the result. This tests FilmCraft's conversion policy, not Avid render cadence.

The Phase 6 full JSON report exposed avoidable allocation: the adapter built a DOM for unrelated
folder/MDB inventories alongside bin evidence. A typed document envelope now retains the complete
bin array while skipping those unused top-level values. Their JSON syntax is still validated;
malformed skipped data is rejected. Both metadata and sequence planning use that envelope.
No identity, source-range, availability or omission policy changed.

Measurements on this development machine, optimized development build:

| Scope | Result |
|---|---|
| Input catalog / saved native project | 112,953,672 / 16,970,924 bytes |
| Earlier full-DOM test process peak RSS | 792,035,328 bytes (755 MiB), one baseline run |
| Filtered-envelope test process peak RSS | 600,588,288–601,145,344 bytes (573 MiB), three final runs; about 24% lower |
| Metadata import | 437–471 ms across those three final runs |
| Native serialization / decode | 28–29 ms / 39–40 ms |
| Output checks | 1,953 masters; 1,761 fully represented / 192 partial; absent A1 and present A2 preserved; one undo step; equal native roundtrip |

RSS is measured externally with `/usr/bin/time -l` over the isolated test process, including
import, assertions and native roundtrip. Times exclude Python, media scanning and media decoding;
they are local observations, not a portable benchmark or a Python-decoder improvement claim.

The new native `avid_bin` module only indexes container/object framing and the observed ABIN
membership prefix. Borrowed payloads preserve unknown classes and fields. It checks the 256 MiB
input cap, one-million-object cap, byte lengths, root/member references and exact object-stream
consumption. Root settings and all semantic object payloads remain opaque. A future membership
variant returns an explicit error; the read-only engine query can still report container coverage
with that diagnostic. It never imports project items or labels structural coverage as full decoding.

Validation:

- 580 engine unit tests pass (15 opt-in); 38 interchange unit cases pass with the private native
  corpus enabled. The separate combined-import corpus passes all 13 cases with Phases 9–12,
  and the Phase 4–6 metadata corpus passes.
- External pyavb 1.4.0 comparison agrees on 25 AVBs / 110,429 objects, bin UIDs, placement counts
  and every membership reference. Phase 6 contains 108,338 objects and 6,934 members, of which
  1,953 are user-placed. No decoder source was read/adapted for the new native module.
- Direct CLI inspection of Phase 6 passes (about 0.01 seconds at the timer's resolution, 46 MiB
  peak process RSS). This reads only framing/membership, so it is not comparable to full semantic
  catalog import or Python decoding. It establishes no native semantic parity.
- Synthetic coverage includes both byte orders, raw non-ASCII strings, truncation/count/length
  mutations, dangling references, unknown versions/classes and arbitrary bytes. New query tests
  verify hostile parameters and unchanged project, state and history. All real samples are
  little-endian; legacy/big-endian real-file compatibility remains unverified.
- Engine/interchange all-target Clippy, CLI build, layers and WASM (44 crates) pass. All 25 AVB
  hashes and the ten Phase 12 source hashes are unchanged (one AVB is in both sets).

Evidence and outputs stay under ignored `target/avid-native-*`, `target/avid-large-*` and
`target/avid-stress-*`. Phase 13 is explicitly excluded and deferred until after the initial PR.
Next are native semantic object decoding/parity and the cancellable File-menu workflow. No new
manual Avid test is needed for this increment. Full release gates remain outstanding, including
the previously recorded unrelated GPU precision failure. No commit, push or submission occurred.

## Native semantics and single-action desktop import (2026-10-11)

The native reader now decodes the tested core objects, managed source descriptors/locators and
sequence wrappers. External pyavb 1.4.0 public-API comparison agrees on all supported properties
of 95,990 core objects and 14,269 additional location/descriptor/effect objects across the explicit
25-bin manifest. Unknown classes/extensions remain explicit. No third-party decoder implementation
was read or adapted. Real big-endian files and comprehensive effect interpretation remain unverified.

File > Import Avid Bin prepares a snapshot in a cancellable job and applies it as one undoable edit.
It attempts clips and sequences together, retaining failed media as metadata and known unsupported
sequence intervals as labeled offline placeholders. Equal-duration/rate single-input wrappers may
be bypassed with diagnostics. A failed track can fall back to a whole-sequence placeholder only
when the composition clock and duration are known. Problems remain visible after import and are
retained in project metadata. Strict catalog conversion remains available for comparison tests.

Audit fixes include resolving placeholder motion coordinates before save (otherwise auto-position
NaNs prevented reopening), maintaining separate unscanned/absent evidence, validating native source
descriptors before binding, and comparing original bytes again after preparation. A changed project
invalidates the prepared snapshot. Optional media search includes subfolders and applies only
unique validated identities from a complete scan. Multiple disjoint roots remain a UI follow-up.

Validation recorded in ignored target logs:

- 586 engine tests pass; 19 private/environment-dependent cases were excluded from that run.
  Separate native corpus checks pass, including Phase 12 parity with the catalog path, six linked
  components, Phases 9–12 offline reconstruction, placeholders, save/reopen and undo/redo.
- 45 interchange cases pass with the private oracle corpus enabled; ten media UI cases pass.
  Targeted all-target Clippy, layering, WASM (44 crates), formatting and desktop build pass.
- The actual desktop single-action preview and successful Phase 12 import were rendered and
  inspected. All 25 AVB hashes and ten Phase 12 source hashes remain unchanged.
- Native Phase 6 metadata-only import measured 74 ms, with 78,249,984 bytes peak process RSS
  including assertions and roundtrip. This scope excludes full graph/media decoding and Python;
  it is not comparable to the earlier full-catalog memory measurement.
- An additional opt-in Phase 6 default-import check retains all 1,953 masters and roundtrips,
  but only five bind offline; 1,948 are retained with binding errors (1,708 exact timing,
  192 managed-source evidence, 46 descriptor range, two track-slot cases). Current folder search
  follows binding and cannot yet recover those failed bindings using MXF evidence. This remains
  a playback limitation, not a successful large-library playback acceptance check.

Phase 13 remains excluded. Full release gates and the previously recorded GPU precision failure
remain outstanding. No commit, push or GitHub submission occurred.

The direct-import report now uses a viewport sized to the window, with action buttons outside
the scroll area. Media matches, technical reader diagnostics and conversion notes have separate
collapsible sections; diagnostics alone are not presented as failed imports. A UI regression opens
all sections with 100 long diagnostics and 40 notes, then adds 100 conversion problems, at
1600×980, 1024×768 and 800×600. Import/Cancel and the post-import Close action remain on screen;
the rendered 800×600 result was inspected. The existing native preview/cancel/apply test also passes.

## Follow-up audit of native import and recovery (2026-10-11)

Reviewed native preparation/commit, source stability and cancellation, semantic graph expansion,
best-effort sequence recovery, identity-based folder linking and the bounded report UI. Fixed:

- Sequence recovery could turn an inactive composition into a live placeholder. Eligibility now
  precedes recovery. Inactive entries are reported and skipped; they do not conflict with live IDs.
- A malformed composition identity/name or duplicate live composition could block unrelated
  sequences. Best-effort planning isolates these entries and rejects every ambiguous live copy.
  Sequence preflight now uses sequence-mode catalog validation, so metadata-only omission text
  cannot fail first on an invalid sequence name. Strict import still refuses invalid batches.
- Native graph accounting charged a fixed node cost but omitted variable component text payloads.
  Repeated references could therefore exceed the intended expansion budget. Variable payloads
  are now charged before decoding/copying, as descriptors already were. A regression exercises
  repeated text-heavy component expansion with a deliberately small byte budget.

The sequence-isolation regression failed before the fixes (zero valid sequences imported), then
passed with the valid live sequence retained and all five problematic entries reported. Save/reopen
preserves the result without resurrecting inactive history as placeholder media.

Validation: 587 engine tests pass (20 opt-in cases excluded); 46 interchange tests pass with the
explicit private oracle manifest. The eight native import checks also pass, including Phase 6
retention and Phase 9–12 offline/linked conversion. Eleven media UI tests, targeted all-target
Clippy, formatting/whitespace checks and the desktop build pass. All 25 AVB
hashes and ten Phase 12 source hashes are unchanged. The known large-library binding limitations,
multiple-root UI follow-up and outstanding full-release GPU gate remain; this is not a claim of
complete AVB support. Phase 13 was not inspected, and nothing was committed or published.

Local evidence: ignored `target/avid-audit-*` logs.

## Multiple media roots (2026-10-11)

The native import dialog now has `+ Add folder`, per-row Browse and additional-row Remove actions.
Folder edits invalidate the reviewed snapshot. Blank rows are omitted; an empty list retains offline
import. The list is capped at 32 roots and scrolls independently of the bounded report; controls
remain accessible at 800×600 with 32 rows and a long error report. Actual test renders were inspected.

`file.prepareAvidBin` accepts `mediaFolders`; the legacy `mediaFolder` parameter remains supported,
but callers cannot supply both. Searches build one complete identity index across all roots, sharing
the existing entry/directory limits and cancellation checks. Canonical directory keys deduplicate
repeated, aliased and overlapping selections, while retained paths identify distinct copies. A root
that disappears during preparation makes the search incomplete and prevents automatic linking.

Synthetic regression coverage splits A1/A2 across independent directory trees, checks playback
channel order after linking, exercises both parent/child selection orders and repeated roots,
preserves ambiguity for distinct copies, and checks a missing root causes no partial automatic edit.
Native Phase 12 also passes with a repeated root in the new array API. Validation includes nine
component-relink cases, eight native import cases, the UI suite, targeted all-target Clippy and
formatting/whitespace checks. The existing binding-before-search order is unchanged; resolving the
large-library descriptor/timing failures from MXF evidence is still the next implementation step.
Local evidence is under ignored `target/avid-multiroot-*`; no samples or media were modified.

## Checkpoint GPU gate investigation (2026-10-11)

The unchanged release GPU comparison failed again for inverted, zero-softness Extract:
maximum relative difference `1.33e-4`, above the existing `1e-4` tolerance. At the user's
request this was corrected before committing, rather than exempted from the gate.

Extract substituted a `0.0001`-wide fade for zero softness. Besides amplifying CPU/GPU
sRGB rounding, this produced fractional grey values at a setting that should have a hard
cutoff. Both implementations now handle nonpositive softness explicitly, avoiding division
by zero. The strict cutoff endpoints retain the feathered function's limiting values;
positive-softness calculations are unchanged. This intentionally removes the old tiny fade
at zero softness; it is not a claim of bit-identical output at those edge pixels.

A new CPU regression first reproduced an unwanted `0.49978495` edge value, then passed
for both cutoffs, inversion and premultiplied alpha. GPU coverage checks binary masks,
complementary inversion, default bounds, equal/reversed bounds and feathered settings.
All 33 GPU tests and 183 renderer tests pass in the targeted debug run (two existing
renderer tests are ignored); the 33 GPU tests also pass in release. The follow-up review
added default-bound and complementary-mask checks, which pass in debug. No tolerance was
relaxed. Full checkpoint gate results are recorded separately when complete.

Local evidence: ignored `target/avid-checkpoint-gpu*`, `target/avid-checkpoint-extract-*`
and `target/gpu-extract-audit.log`. The GPU correction is kept separate from the Avid feature.

The full release test sweep then reported 2,877 passing tests, four failures and 62 ignored
tests. Three failures identified eight new Avid menu/relink strings missing from the Spanish
and Simplified Chinese catalogs; the missing entries were added with their placeholders intact.
The fourth reproduced a pre-existing caption font-picker automation defect: a partly clipped
font row exposed its full rectangle, so clicking its center could miss the visible popup.
Its automation rectangle now intersects the UI clip rectangle. The existing font-selection
test passes after that correction; font loading and assets were not changed. Both pre-existing
gate corrections are grouped in a maintenance commit separate from the Avid feature. Full dialog localization remains
on the pre-submission list; passing catalog coverage does not imply every plain UI label is localized.

A subsequent release run exposed a timing assumption in the component-relink UI test. It
waited for the search job to finish, then immediately queried automation labels from the
previous frame. The parallel suite reproduced a stale `missing` label even though an isolated
run passed. The wait helper now requires a completed search report and advances the registry
before inspecting labels; product behavior and assertions remain unchanged. All 12 media UI
tests then passed in ten consecutive parallel release runs. Evidence: ignored
`target/avid-checkpoint-media-{repeat-before,after,repeat-after}.log`.

The final `cargo xtask ci` completed successfully: formatting, workspace/all-target release
Clippy with warnings denied, 2,881 passing release tests (zero failures, 62 ignored), dependency
layering, attribution for 69 assets and the WASM check across 44 crates. This run includes the
final GPU coverage and both UI corrections. Ignored private-corpus tests were exercised in the
separate targeted runs above; the full gate does not implicitly rerun them. Local evidence:
`target/avid-checkpoint-ci-green.log`.
