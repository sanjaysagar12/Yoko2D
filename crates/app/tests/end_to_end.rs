// End-to-end integration test for the file write -> OS event -> debounce ->
// channel -> resync -> new geometry pipeline. Lives under tests/ (rather
// than app's own #[cfg(test)] modules) because it exercises the real
// filesystem watcher together with PatternSync, spanning both watch.rs and
// sync.rs rather than testing either in isolation.

use std::time::Duration;

#[test]
fn file_change_flows_through_watcher_into_resynced_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measurements.json");
    std::fs::write(
        &path,
        r#"{"measurements":[{"name":"height_scapula","value":40.0}]}"#,
    )
    .unwrap();

    let mut doc = core_lib::Document::default();
    let a = doc.add_base_point("A", 0.0, 0.0);
    let a1 = doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();

    let mut sync = app::sync::PatternSync::new(doc, path.clone()).unwrap();
    // PatternSync::new already resolved against the file's initial contents.
    let initial_point = sync.current_data().get_point(a1).unwrap();
    assert!((initial_point.x - 4.0).abs() < 1e-9);

    let (_handle, receiver) =
        app::watch::spawn_watcher(path.clone(), Duration::from_millis(100)).unwrap();

    std::fs::write(
        &path,
        r#"{"measurements":[{"name":"height_scapula","value":90.0}]}"#,
    )
    .unwrap();

    let event = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(event.path, path);

    sync.resync().unwrap();

    let updated_point = sync.current_data().get_point(a1).unwrap();
    assert!((updated_point.x - 9.0).abs() < 1e-9);
    assert!((updated_point.y - 0.0).abs() < 1e-9);
}
