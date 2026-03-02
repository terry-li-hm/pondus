/// Smoke tests — run the actual binary against cached data.
/// These catch crashes and CLI regressions, not source format changes.
/// Requires: pondus has been run at least once so the cache is populated.
use std::process::Command;

fn pondus() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pondus"))
}

#[test]
fn sources_exits_zero() {
    let out = pondus().arg("sources").output().expect("failed to run");
    assert!(out.status.success(), "pondus sources failed: {:?}", out);
}

#[test]
fn rank_exits_zero() {
    let out = pondus().arg("rank").output().expect("failed to run");
    assert!(out.status.success(), "pondus rank failed: {:?}", out);
}

#[test]
fn rank_aggregate_exits_zero() {
    let out = pondus()
        .args(["rank", "--aggregate"])
        .output()
        .expect("failed to run");
    assert!(
        out.status.success(),
        "pondus rank --aggregate failed: {:?}",
        out
    );
}

#[test]
fn rank_aggregate_has_results() {
    let out = pondus()
        .args(["rank", "--aggregate"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // JSON output should contain at least one ranked model
    assert!(
        stdout.contains("avg_percentile"),
        "rank --aggregate returned no scored models"
    );
}

#[test]
fn check_known_model_finds_results() {
    let out = pondus()
        .args(["check", "claude-sonnet-4.6"])
        .output()
        .expect("failed to run");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should find at least one source with scores
    assert!(
        stdout.contains("intelligence_index")
            || stdout.contains("elo")
            || stdout.contains("resolved_rate")
            || stdout.contains("score"),
        "check claude-sonnet-4.6 found no metrics"
    );
}

#[test]
fn check_unknown_model_exits_zero_with_warn() {
    // A typo should not crash — it exits 0 and emits a stderr warning
    let out = pondus()
        .args(["check", "this-model-does-not-exist-xyz"])
        .output()
        .expect("failed to run");
    assert!(out.status.success(), "check with unknown model should exit 0");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[warn]"),
        "expected no-match warning on stderr, got: {stderr}"
    );
}

#[test]
fn rank_tag_reasoning_exits_zero() {
    let out = pondus()
        .args(["rank", "--tag", "reasoning"])
        .output()
        .expect("failed to run");
    assert!(
        out.status.success(),
        "pondus rank --tag reasoning failed: {:?}",
        out
    );
}

#[test]
fn rank_invalid_tag_exits_nonzero() {
    let out = pondus()
        .args(["rank", "--tag", "notarealtag"])
        .output()
        .expect("failed to run");
    assert!(
        !out.status.success(),
        "invalid tag should exit non-zero"
    );
}
