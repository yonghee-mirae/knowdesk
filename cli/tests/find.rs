//! Black-box integration tests for `kdfind` (`docs/13_CLI_Tool.md`). Runs the
//! actual compiled binary (`CARGO_BIN_EXE_kdfind`, provided by Cargo for
//! integration tests in the same package) against a temporary sample folder, so
//! these exercise the full in-memory index → search → print pipeline exactly as
//! a user would invoke it.
//!
//! Each test gets its own `XDG_DATA_HOME`, so `settings_cli.json` (auto-created
//! on first run) never leaks between tests or touches the real home directory.

use std::path::Path;
use std::process::Command;

fn run_raw(sample_dir: &Path, data_home: &Path, args: &[&str]) -> std::process::Output {
    let output = Command::new(env!("CARGO_BIN_EXE_kdfind"))
        .arg(sample_dir)
        .args(args)
        // `kdfind` must never read these - set to obviously-invalid paths so a
        // regression (falling back to the environment like `knowdesk-cli` does)
        // would surface as a crash/failure rather than passing by accident.
        .env("KNOWDESK_KIWI_LIB_PATH", "/nonexistent/libkiwi.so")
        .env("KNOWDESK_KIWI_MODEL_DIR", "/nonexistent/model")
        .env("KNOWDESK_PDFIUM_LIB_DIR", "/nonexistent/pdfium")
        .env("XDG_DATA_HOME", data_home)
        .output()
        .expect("failed to run kdfind");
    assert!(
        output.status.success(),
        "kdfind exited with {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn run(sample_dir: &Path, data_home: &Path, args: &[&str]) -> String {
    String::from_utf8(run_raw(sample_dir, data_home, args).stdout).unwrap()
}

/// Same as `run`, but also returns stderr - for tests asserting on a warning
/// notice rather than (or in addition to) the search results themselves.
fn run_with_stderr(sample_dir: &Path, data_home: &Path, args: &[&str]) -> (String, String) {
    let output = run_raw(sample_dir, data_home, args);
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn keyword_search_finds_the_expected_file_and_not_an_unrelated_one() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    std::fs::write(
        sample_dir.path().join("규정.txt"),
        "채권 발행 절차에 대한 이사회 결의 사항을 정리한다.",
    )
    .unwrap();
    std::fs::write(
        sample_dir.path().join("무관.txt"),
        "오늘 점심 메뉴는 김치찌개였다.",
    )
    .unwrap();

    let stdout = run(
        sample_dir.path(),
        data_home.path(),
        &["채권", "AND", "발행"],
    );

    assert!(stdout.contains("규정.txt"), "stdout: {stdout}");
    assert!(stdout.contains("[exact match]"), "stdout: {stdout}");
    assert!(!stdout.contains("무관.txt"), "stdout: {stdout}");
}

#[test]
fn extension_filter_narrows_results_to_the_matching_extension() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    std::fs::write(sample_dir.path().join("보고서.txt"), "채권 발행 보고서").unwrap();
    std::fs::write(sample_dir.path().join("보고서.md"), "채권 발행 보고서").unwrap();

    let stdout = run(sample_dir.path(), data_home.path(), &["채권", "x:txt"]);

    assert!(stdout.contains("보고서.txt"), "stdout: {stdout}");
    assert!(!stdout.contains("보고서.md"), "stdout: {stdout}");
}

#[test]
fn filename_mode_matches_by_filename_even_when_the_content_does_not_contain_the_term() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    std::fs::write(
        sample_dir.path().join("이사회결의.txt"),
        "오늘 점심 메뉴는 김치찌개였다.",
    )
    .unwrap();

    // Content mode: the filename word never appears in the body, so no hit.
    let content_stdout = run(sample_dir.path(), data_home.path(), &["이사회결의"]);
    assert!(
        content_stdout.contains("No results"),
        "stdout: {content_stdout}"
    );

    // Filename mode (`-f`): matches by filename substring instead.
    let filename_stdout = run(sample_dir.path(), data_home.path(), &["-f", "이사회결의"]);
    assert!(
        filename_stdout.contains("이사회결의.txt"),
        "stdout: {filename_stdout}"
    );
}

#[test]
fn limit_caps_the_number_of_results() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    // Distinct content per file - documents are deduped by content hash
    // (`DocId = SHA256(Content)`), so identical bodies would collapse into a
    // single document/hit regardless of `--limit`.
    for i in 0..5 {
        std::fs::write(
            sample_dir.path().join(format!("문서{i}.txt")),
            format!("채권 발행 {i}"),
        )
        .unwrap();
    }

    let stdout = run(sample_dir.path(), data_home.path(), &["-l", "2", "채권"]);

    let hit_count = stdout.matches("[exact match]").count();
    assert_eq!(hit_count, 2, "stdout: {stdout}");
}

/// Flags must precede the query - once the query starts, a flag-looking word
/// (like `-l`) is deliberately treated as literal query text instead (so a
/// document genuinely containing "-l" is still findable), and `kdfind` warns
/// on stderr that this is what happened rather than failing silently.
#[test]
fn limit_flag_after_the_query_is_treated_as_literal_text_with_a_warning() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    for i in 0..5 {
        std::fs::write(
            sample_dir.path().join(format!("문서{i}.txt")),
            format!("채권 발행 {i}"),
        )
        .unwrap();
    }

    // "-l" became part of the query text, so it's no longer just "채권" (which
    // would match all 5 files) - none of them contain the literal word "-l".
    let (stdout, stderr) =
        run_with_stderr(sample_dir.path(), data_home.path(), &["채권", "-l", "2"]);

    assert!(stdout.contains("No results"), "stdout: {stdout}");
    assert!(
        stderr.contains("-l") && stderr.contains("looks like a flag"),
        "stderr: {stderr}"
    );
}

/// Same as above, for `-f`.
#[test]
fn filename_flag_after_the_query_is_treated_as_literal_text_with_a_warning() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    std::fs::write(
        sample_dir.path().join("이사회결의.txt"),
        "오늘 점심 메뉴는 김치찌개였다.",
    )
    .unwrap();

    // Content mode (the "-f" never switched it to filename mode) and the
    // filename word never appears in the body, so no hit either way.
    let (stdout, stderr) =
        run_with_stderr(sample_dir.path(), data_home.path(), &["이사회결의", "-f"]);

    assert!(stdout.contains("No results"), "stdout: {stdout}");
    assert!(
        stderr.contains("-f") && stderr.contains("looks like a flag"),
        "stderr: {stderr}"
    );
}

/// Flags placed before the query keep working exactly as before - this is the
/// supported, unambiguous order.
#[test]
fn flags_before_the_query_still_work_without_any_warning() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    std::fs::write(
        sample_dir.path().join("이사회결의.txt"),
        "오늘 점심 메뉴는 김치찌개였다.",
    )
    .unwrap();

    let (stdout, stderr) =
        run_with_stderr(sample_dir.path(), data_home.path(), &["-f", "이사회결의"]);

    assert!(stdout.contains("이사회결의.txt"), "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn first_run_creates_a_default_settings_cli_json() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    std::fs::write(sample_dir.path().join("규정.txt"), "채권").unwrap();

    run(sample_dir.path(), data_home.path(), &["채권"]);

    let settings_path = data_home.path().join("KnowDesk").join("settings_cli.json");
    assert!(
        settings_path.is_file(),
        "expected {settings_path:?} to be created"
    );
    let text = std::fs::read_to_string(&settings_path).unwrap();
    assert!(
        text.contains("enable_morphological_analysis"),
        "text: {text}"
    );
}

#[test]
fn version_flag_prints_the_version_and_exits_successfully() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();

    let short = run(sample_dir.path(), data_home.path(), &["-v"]);
    assert!(
        short.trim() == format!("kdfind {}", env!("CARGO_PKG_VERSION")),
        "stdout: {short:?}"
    );

    // `--version` (long form) must print the exact same thing.
    let long = run(sample_dir.path(), data_home.path(), &["--version"]);
    assert_eq!(short, long);
}

#[test]
fn help_lists_the_version_flag() {
    let sample_dir = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();

    let stdout = run(sample_dir.path(), data_home.path(), &["--help"]);
    assert!(stdout.contains("-v, --version"), "stdout: {stdout}");
}
