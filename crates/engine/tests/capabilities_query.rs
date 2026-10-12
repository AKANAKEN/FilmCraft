//! A separate process keeps the deliberately hostile probe out of other engine tests.

#[test]
fn capabilities_query_never_invokes_registered_driver_probes() {
    fn forbidden_probe() -> bool {
        panic!("capabilities must not probe drivers")
    }
    filmcraft_export::register_format_probe(filmcraft_export::Format::Av1, forbidden_probe);
    let mut session = filmcraft_engine::Session::default();
    let before = filmcraft_export::hw_encode_stats();
    let value = session.execute("perf.stats", serde_json::json!({})).unwrap();
    assert_eq!(value["capabilities"]["scope"], "process");
    assert_eq!(filmcraft_export::hw_encode_stats().sessions, before.sessions);
    assert!(value["capabilities"]["encoder"]["latest"].is_null());

    // Exercise the real selection boundary with an injected hardware decline and the built-in
    // software encoder. Initialization does not write the output or read any external media.
    fn declined_hardware(
        format: filmcraft_export::Format,
        _: u32,
        _: u32,
        _: filmcraft_time::FrameRate,
        settings: &filmcraft_export::ExportSettings,
    ) -> Option<filmcraft_export::Result<Box<dyn filmcraft_export::VideoEncoder>>> {
        if format == filmcraft_export::Format::H264 && settings.hardware_encoding == filmcraft_export::HardwareEncoding::Auto {
            filmcraft_export::diagnostics::note_encoder_declined("test hardware", "synthetic driver absent");
        }
        None
    }
    filmcraft_export::register_encoder(declined_hardware);
    filmcraft_export::diagnostics::note_encoder_declined("unrelated probe", "must not contaminate a selection");
    session.execute("file.openDemoProject", serde_json::json!({})).unwrap();
    let sequence = session.state.active_sequence.unwrap();
    let mut settings = filmcraft_export::ExportSettings {
        format: filmcraft_export::Format::H264,
        include_audio: false,
        frame_size: Some((16, 16)),
        gpu_rendering: filmcraft_export::GpuRendering::Off,
        hardware_encoding: filmcraft_export::HardwareEncoding::Auto,
        ..Default::default()
    };
    let exporter = filmcraft_export::Exporter::new(session.project.clone(), sequence, &settings, &filmcraft_export::Progress::default()).unwrap();
    let selected = filmcraft_engine::perf::capabilities_json();
    let latest = &selected["encoder"]["latest"];
    assert_eq!(latest["format"], "h264");
    assert_eq!(latest["backend"], "FilmCraft");
    assert_eq!(latest["kind"], "software");
    assert_eq!(latest["fallbackReason"], "test hardware: synthetic driver absent");
    drop(exporter);
    settings.hardware_encoding = filmcraft_export::HardwareEncoding::Off;
    let _exporter = filmcraft_export::Exporter::new(session.project.clone(), sequence, &settings, &filmcraft_export::Progress::default()).unwrap();
    let selected = filmcraft_engine::perf::capabilities_json();
    assert_eq!(selected["encoder"]["latest"]["fallbackReason"], "Hardware encoding is off for this export.");
}
