use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn prints_placeholder_and_exits_zero() {
    // Moved to its own `formula-check` subcommand now that `run` exists;
    // invoked explicitly here instead of relying on no-argument default
    // behavior, which now prints usage and exits non-zero instead.
    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .arg("formula-check")
        .assert()
        .success()
        .stdout(predicate::str::contains("yoko2d-cli placeholder"));
}
