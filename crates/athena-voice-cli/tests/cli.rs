use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_shows_serve_subcommand() {
    Command::cargo_bin("athena-voice")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("serve"));
}

#[test]
fn serve_help_shows_dry_run_flag() {
    Command::cargo_bin("athena-voice")
        .unwrap()
        .args(["serve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--config"));
}
