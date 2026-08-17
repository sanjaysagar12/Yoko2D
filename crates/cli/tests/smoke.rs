use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn prints_placeholder_and_exits_zero() {
    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .assert()
        .success()
        .stdout(predicate::str::contains("yoko2d-cli placeholder"));
}
