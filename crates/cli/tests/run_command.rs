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
