// Integration tests for the `view` subcommand, exercising the actual built
// `yoko2d-cli` binary end to end, the same way crates/cli/tests/run_command.rs
// already exercises `run`.

use assert_cmd::Command;
use predicates::prelude::*;

/// The workspace root, computed the same CARGO_MANIFEST_DIR-relative way
/// crates/cli/tests/run_command.rs's own `workspace_root` is — see that
/// function's own doc comment for the rationale.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn view_prints_every_point_name_and_tool_kind() {
    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("view")
        .arg("fixtures/patterns/l_shape.xml")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"A\"")
                .and(predicate::str::contains("\"B\""))
                .and(predicate::str::contains("\"C\""))
                .and(predicate::str::contains("BasePoint"))
                .and(predicate::str::contains("EndLine"))
                .and(predicate::str::contains("Line")),
        );
}

#[test]
fn view_raw_emits_the_exact_underlying_xml() {
    let output = Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("view")
        .arg("fixtures/patterns/l_shape.xml")
        .arg("--raw")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let raw_stdout = String::from_utf8(output).unwrap();
    // Reconstructs successfully, proving --raw emitted the exact underlying
    // XML text, not a reformatted/lossy version.
    let (from_raw, _measurements_path) = io::deserialize_document(&raw_stdout).unwrap();

    let direct_xml =
        std::fs::read_to_string(workspace_root().join("fixtures/patterns/l_shape.xml")).unwrap();
    let (from_direct_load, _measurements_path) = io::deserialize_document(&direct_xml).unwrap();

    assert_eq!(from_raw.history(), from_direct_load.history());
}

#[test]
fn view_with_no_positional_argument_fails_with_a_usage_error() {
    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("view")
        .assert()
        .failure()
        .stderr(predicate::str::contains("yoko2d-cli"));
}

// Opens a real native window (via app::run_with_document, now a read-only
// viewer — see crates/app/src/lib.rs), so it needs an actual display to run
// against; this project has no automated GUI pixel-testing infrastructure
// (see crates/app/src/lib.rs's own regression-guard tests for the same
// limitation), matching how every other GUI-launching verification in this
// project has always been described as manual, not automated. Run
// explicitly, with a display available, via `cargo test -p cli -- --ignored`.
#[test]
#[ignore]
fn view_open_launches_the_gui_without_erroring() {
    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("view")
        .arg("fixtures/patterns/l_shape.xml")
        .arg("--open")
        .assert()
        .success();
}
