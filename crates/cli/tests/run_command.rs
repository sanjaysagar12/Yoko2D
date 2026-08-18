// Integration tests for the `run` subcommand, exercising the actual built
// `yoko2d-cli` binary end to end (action script -> Document -> recompute ->
// output JSON), the same way crates/cli/tests/smoke.rs already exercises
// `formula-check`.

use assert_cmd::Command;
use predicates::prelude::*;

/// The workspace root, computed the same CARGO_MANIFEST_DIR-relative way
/// every other test fixture path in this repo is resolved (see e.g.
/// crates/io/src/measurements.rs's own tests). Every path this file passes
/// to the CLI — both the positional script path and its own internal
/// `measurements_path` — is resolved relative to the spawned process's
/// working directory, so tests here explicitly set that directory to the
/// workspace root via `.current_dir(..)`, matching how a real user would
/// invoke this binary from the repo root.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn run_with_output_flag_writes_expected_point_and_line_data() {
    let dir = tempfile::tempdir().unwrap();
    let output_path = dir.path().join("output.json");

    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("run")
        .arg("fixtures/actions/simple_line.json")
        .arg("--output")
        .arg(&output_path)
        .assert()
        .success();

    let contents = std::fs::read_to_string(&output_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();

    let points = parsed["points"].as_array().unwrap();
    let lines = parsed["lines"].as_array().unwrap();
    assert!(!points.is_empty()); // at least the BasePoint the fixture script defines
    assert!(!lines.is_empty()); // the add_line action the fixture script defines

    // The fixture's BasePoint "A" is always at the literal origin.
    let origin = points
        .iter()
        .find(|p| p["x"].as_f64() == Some(0.0) && p["y"].as_f64() == Some(0.0));
    assert!(origin.is_some());
}

#[test]
fn run_with_export_image_flag_writes_a_nonempty_png_file() {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("output.png");

    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("run")
        .arg("fixtures/actions/l_to_rectangle.json")
        .arg("--export-image")
        .arg(&image_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("exported image to"));

    let metadata = std::fs::metadata(&image_path).unwrap();
    assert!(metadata.len() > 0); // a real PNG was actually written, not an empty file
}

#[test]
fn run_on_a_nonexistent_script_fails_with_a_clear_stderr_message() {
    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("run")
        .arg("this/path/definitely/does/not/exist.json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("yoko2d-cli"));
}

// Proves the full CLI-PROCESS round trip (not just the library function
// execute_action_script/action_script.rs's own tests already cover): the
// real built binary, given the l_shape -> rectangle action script, writes a
// pattern file via --save-pattern that, read back independently, has the
// same 8-record structure and recomputes to the same closed-rectangle
// coordinates as crates/cli/src/action_script.rs's own end-to-end test.
#[test]
fn run_with_save_pattern_writes_a_loadable_pattern_file_with_the_expected_structure() {
    let dir = tempfile::tempdir().unwrap();
    let saved_path = dir.path().join("rectangle.xml");

    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("run")
        .arg("fixtures/actions/l_to_rectangle.json")
        .arg("--save-pattern")
        .arg(&saved_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("saved pattern to"));

    let xml = std::fs::read_to_string(&saved_path).unwrap();
    let (document, _measurements_path) = io::deserialize_document(&xml).unwrap();
    assert_eq!(document.history().len(), 8); // 5 original L-shape records + 3 additively added ones

    let data = core_lib::recompute_all(&document).unwrap();
    let point_id = |name: &str| {
        document
            .history()
            .iter()
            .find(|r| match &r.kind {
                core_lib::ToolKind::BasePoint { name: n, .. } => n == name,
                core_lib::ToolKind::EndLine { name: n, .. } => n == name,
                _ => false,
            })
            .unwrap()
            .id
    };

    let a = data.get_point(point_id("A")).unwrap();
    let b = data.get_point(point_id("B")).unwrap();
    let c = data.get_point(point_id("C")).unwrap();
    let d = data.get_point(point_id("D")).unwrap();
    assert!((a.x - 0.0).abs() < 1e-9 && (a.y - 0.0).abs() < 1e-9);
    assert!((b.x - 10.0).abs() < 1e-9 && (b.y - 0.0).abs() < 1e-9);
    assert!((c.x - 10.0).abs() < 1e-9 && (c.y - 5.0).abs() < 1e-9);
    assert!((d.x - 0.0).abs() < 1e-9 && (d.y - 5.0).abs() < 1e-9);
}
