# FilmCraft interface

The egui frontend draws panels and dialogs, and dispatches project changes through the engine.
Interactive controls keep stable automation ids regardless of the interface language.

## Interface appearance

Settings > Appearance and View > Appearance > Sync with system select the saved Auto mode.
The header cycles Auto, Light and Dark. Menu checkmarks reflect the saved mode, including when
Auto displays a light or dark palette. Manual Darkest, Dark and Light choices ignore desktop
changes, and new users keep Darkest. Legacy `colorTheme` files migrate to their fixed theme;
files with an explicit `appearanceMode` retain it. Auto falls back to its saved dark theme when
no system appearance is available. Appearance changes do not alter video or project colors.

Native windows keep inherited appearance in every mode so macOS and Windows theme events remain
available when returning to Auto. Both egui style branches contain FilmCraft's resolved palette,
including custom panels, the timeline ruler and controls. Native window decorations may follow
the desktop independently of a manual interface palette. Linux follows desktop portal signals;
without a working portal, its one-shot `gsettings` fallback supplies only a startup reading.
The web frontend uses the browser appearance report.

`theme::tests::palettes_keep_native_tracking_and_both_style_branches` checks emitted native
commands and both styles. Settings tests cover direct System selection and menu checks;
appearance frame tests cover transitions, manual modes and preferences. Injected appearance
inputs verify the resolver and command output, not native OS event delivery.

## Transition drop previews

Dragging a video or audio transition over a compatible clip shows its actual time span, the
selected cut and an In/Out label. Shared cuts center the span; isolated edges use a one-sided
span. The preview uses the engine's read-only transition planner, including Timeline duration
preferences, and release dispatches `effects.apply` with the same edge. Locked tracks and
incompatible clip types show no valid transition preview. Ordinary effects retain their clip
outline. `timeline.transitionDropPreview` exposes the span and edge to automation.

`transition_drop_ui` covers both edges of short video/audio clips, hover without project edits,
the committed span and undo. Set `FILMCRAFT_UI_SNAPSHOT_DIR` to render screenshots with wgpu.

## Localisation

Edit > Language offers English, Japanese, Spanish, Brazilian Portuguese, Ukrainian, Simplified
Chinese, German, Russian and French. The language is stored in the engine's `general.interfaceLanguage` preference,
restored on startup, and also available in Settings > General.
Its default, System Language (`system`), follows the operating system: the first of the user's
preferred languages that the interface has (the host supplies them through
`HostHooks::system_languages`: `sys-locale` on the desktop, `navigator.languages` on the web),
otherwise English.
The `app.language.*` UI commands and `prefs.set` reach it through the control channel.
Japanese requires a craft-fonts build or a suitable installed font.

English source strings are lookup keys in `src/i18n/<code>.tsv`. `tl!` translates literals, `tlf!`
fills translated templates, and `i18n::t` translates names from registries. Placeholder values
(including user filenames containing braces) are inserted literally. Catalog translations are
original work using ordinary language, without proprietary localisation resources.

Spanish, Japanese, Simplified Chinese and French cover menus, panels, dialogs, settings and registry labels. Searches accept both the
translated label and its English source, including Unicode capitals. Project content, command ids
and preference values retain their original values. Engine errors, CLI and MCP messages remain
English. Brazilian Portuguese and Ukrainian currently cover the core menus, German every menu; they
fall back to English elsewhere.

Verification: `cargo test -p filmcraft-ui-egui` checks catalog syntax, duplicate keys, placeholders,
literal/menu/registry coverage and UI behaviour; `cargo xtask ci` runs the workspace gates. Visual
checks use the control channel to switch language and capture the resulting panels.

## Source playback

`source_playback.rs` auditions media independently of Program playback. It reuses MediaSource
decoding, output mapping, the device clock and the desktop play-ahead buffer. Only one monitor
owns the device at a time. Source frame jobs use the existing frame server and Source resolution.
The live project, undo stack and timeline playhead are unchanged. See [monitor documentation](../../docs/monitors.md)
for behavior, commands, limitations and regression tests. Source playback tests use synthetic
demo media and fake audio outputs; no third-party assets are introduced.

Source drag controls snapshot the selected span and dispatch `timeline.place` for video, audio,
or linked video/audio. Full-clip marks are implicit until set by the user. The engine owns edits,
range validation and undo; the UI owns the drag gesture. See `source_drag_ui` and the engine's
`source_placement_tests` for regression coverage.

Source range handles use transient gesture previews and the engine's integer-frame
`source_monitor::adjust_range`; release dispatches one mark edit. Monitor command routing
keeps Source navigation, markers and marked-range playback separate from Program.
