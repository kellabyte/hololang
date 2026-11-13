//! End-to-end contracts for LLVM-free source presentation and formatting.
//!
//! These tests invoke the actual CLI with files/streams and inspect output bytes,
//! style overrides, failure status and file ownership. They validate behaviour,
//! not performance, and never replace compiler work with a host-language kernel.

use holo::driver::TempDir;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// Valid source that needs canonical indentation and vertical header formatting.
const SOURCE: &str = "// Tool fixture: comments and literal spelling survive.\nfunc main returns u64 {\nreturn 0\n}\n";

/// Return a real CLI invocation with native tool discovery deliberately unavailable.
/// Text commands must remain functional without spawning a compiler toolchain.
fn cli() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_holo"));
    command.env("HOLO_LLVM_PREFIX", "/nonexistent/holo-text-tool-test");
    command.env_remove("NO_COLOR");
    command
}

/// Create a source file in a uniquely owned scratch directory, preserving its bytes.
fn fixture(source: &str) -> (TempDir, PathBuf) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("source example.holo");
    fs::write(&path, source).unwrap();
    (directory, path)
}

/// Send a small UTF-8 fixture over stdin, close the stream and collect process output.
/// Fixtures remain below pipe capacity so this helper cannot deadlock on output backpressure.
fn stdin(arguments: &[&str], source: &str) -> Output {
    let mut child = cli()
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Require successful completion while keeping stderr visible in assertion failures.
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Redirected default output is byte-exact, including CRLF, tabs and an absent final newline.
#[test]
fn redirected_highlight_preserves_source_without_ansi_or_chrome() {
    let source = "// π and \t tabs\r\nfunc main returns u64 { return 0 }";
    let (_scratch, path) = fixture(source);
    let output = success(cli().arg("highlight").arg(path).output().unwrap());
    assert_eq!(output, source);
    assert_eq!(success(stdin(&["highlight", "--stdin"], source)), source);
}

/// Explicit terminal presentation is compact and numbers source lines rather than chrome.
#[test]
fn header_numbers_frame_and_custom_palette_are_independent_of_llvm() {
    let (scratch, path) = fixture("func main\n    returns u64\n{ return 0 }\n");
    let stylesheet = scratch.path().join("palette.hss");
    fs::write(
        &stylesheet,
        ".keyword { color: #010203; font-weight: normal; }\n",
    )
    .unwrap();
    let output = success(
        cli()
            .arg("highlight")
            .arg(&path)
            .args([
                "--color=always",
                "--header",
                "--line-numbers",
                "--frame",
                "--style",
            ])
            .arg(stylesheet)
            .output()
            .unwrap(),
    );
    assert!(output.contains(path.to_str().unwrap()));
    assert!(output.contains("Holo"));
    assert!(output.contains("3 lines"));
    assert!(output.contains("╭─"));
    assert!(output.contains("╰─"));
    assert!(output.contains("\x1b[38;2;1;2;3mfunc"));
    assert!(output.contains("1\x1b[0m"));
    assert_eq!(output.matches(" │ ").count(), 3);
}

/// Explicit colour overrides NO_COLOR; never remains absolute even with a custom palette.
#[test]
fn colour_policy_and_control_character_display_are_explicit() {
    let (_scratch, path) =
        fixture("// escape \x1b[31m and \u{009b}\nfunc main returns u64 { return 0 }");
    let colored = success(
        cli()
            .arg("highlight")
            .arg(&path)
            .args(["--plain", "--color=always"])
            .env("NO_COLOR", "1")
            .output()
            .unwrap(),
    );
    assert!(colored.contains("\x1b["));
    assert!(colored.contains("\\x1b[31m"));
    assert!(colored.contains("\\u{9b}"));
    assert!(!colored.contains("escape \x1b[31m"));
    let plain = success(
        cli()
            .arg("highlight")
            .arg(&path)
            .args(["--header", "-n", "--color", "never"])
            .output()
            .unwrap(),
    );
    assert!(!plain.contains('\x1b'));
    assert!(plain.contains("1 │ // escape \\x1b"));
    let auto = success(
        cli()
            .arg("highlight")
            .arg(&path)
            .env("NO_COLOR", "1")
            .output()
            .unwrap(),
    );
    assert_eq!(auto, fs::read_to_string(path).unwrap());
}

/// Empty/unfinished source is displayable; terminal output is not a syntax acceptance gate.
#[test]
fn highlight_handles_empty_and_unfinished_utf8_input() {
    assert_eq!(
        success(stdin(
            &["highlight", "--stdin", "--plain", "--color=always"],
            ""
        )),
        ""
    );
    let source = "/* nested /* π */ still typing";
    let output = success(stdin(
        &["highlight", "-", "--color=always", "--line-numbers"],
        source,
    ));
    assert!(output.contains("nested /* π */ still typing"));
    assert!(output.ends_with('\n'));
}

/// Decorated CRLF display preserves real line boundaries across multiline comment spans.
#[test]
fn coloured_crlf_comments_keep_exact_source_line_boundaries() {
    let source = "/* π\r\n nested /* block */\r\n*/\r\nfunc main returns u64 { return 0 }\r\n";
    let output = success(stdin(
        &[
            "highlight",
            "--stdin",
            "--color=always",
            "--header",
            "--frame",
            "-n",
        ],
        source,
    ));
    assert_eq!(output.matches("\r\n").count(), 4);
    assert_eq!(output.matches(" │ ").count(), 4);
    assert!(!output.contains("\\x0d"));
    assert!(output.contains("nested /* block */"));
    assert!(output.contains("4 lines"));
}

/// Invalid themes and conflicting options fail before source output or accidental file access.
#[test]
fn invalid_highlight_configuration_has_no_partial_output() {
    let (scratch, path) = fixture(SOURCE);
    let stylesheet = scratch.path().join("invalid.hss");
    // Every malformed stylesheet must fail loudly rather than silently ignore a styling typo.
    for invalid in [
        ".keyword { colour: #010203; }",
        ".unknown { color: #010203; }",
        ".type { color: #xyzxyz; }",
        ".type {",
        "/* open",
    ] {
        fs::write(&stylesheet, invalid).unwrap();
        let output = cli()
            .arg("highlight")
            .arg(&path)
            .arg("--style")
            .arg(&stylesheet)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    // Duplicates, unknown values and contradictory preferences reject deterministically.
    for arguments in [
        vec!["--color=bad"],
        vec!["--color=always", "--color=never"],
        vec!["--plain", "--frame"],
        vec!["--header", "--no-header"],
        vec!["--stdin"],
        vec!["--typo"],
    ] {
        let output = cli()
            .arg("highlight")
            .arg(&path)
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

/// Formatting defaults to stdout, has a clean stdin protocol, and supports an idempotent check.
#[test]
fn formatting_stdout_stdin_check_and_explicit_write_share_one_engine() {
    let (_scratch, path) = fixture(SOURCE);
    let expected = holo::format::format(SOURCE).unwrap();
    assert_ne!(expected, SOURCE);
    let output = success(cli().arg("fmt").arg(&path).output().unwrap());
    assert_eq!(output, expected);
    assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
    assert_eq!(success(stdin(&["fmt", "--stdin"], SOURCE)), expected);
    let check = cli().arg("fmt").arg(&path).arg("--check").output().unwrap();
    assert_eq!(check.status.code(), Some(1));
    assert!(check.stdout.is_empty());
    assert!(String::from_utf8_lossy(&check.stderr).contains("needs formatting"));
    success(cli().arg("fmt").arg(&path).arg("--write").output().unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    success(cli().arg("fmt").arg(&path).arg("--check").output().unwrap());
    success(stdin(&["fmt", "--stdin", "--check"], &expected));
}

/// Nested construction arms keep one continuation indent per active arm list.
/// The formatter must also return to entry indentation after the last block.
#[test]
fn formatting_keeps_nested_construct_arm_boundaries() {
    let source = "func main returns u64 {
core.construct(memory, make())
  on ok(record) then {
core.construct(other, make())
  on ok(child) then { core.print(child.field) }
  on error then { core.assert(false) }
  }
  on error then { core.assert(false) }
return 0
}
";
    let formatted = success(stdin(&["fmt", "--stdin"], source));
    assert!(formatted.contains("        on ok(record) then {\n"));
    assert!(formatted.contains("                on ok(child) then {\n"));
    assert!(formatted.contains("                    core.print(child.field)\n"));
    assert!(formatted.contains("\n    return 0\n"));
    success(stdin(&["fmt", "--stdin", "--check"], &formatted));
}

/// Syntax failures and incompatible formatting policies cannot mutate a source file.
#[test]
fn formatting_errors_never_produce_source_output_or_replace_invalid_source() {
    let (_scratch, path) = fixture("func main { /* unterminated");
    let before = fs::read(&path).unwrap();
    let output = cli().arg("fmt").arg(&path).arg("--write").output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(String::from_utf8_lossy(&output.stderr).contains("error:"));
    // Invalid combinations reject before consuming stdin or writing formatted text.
    for arguments in [
        vec!["fmt", "--stdin", "--write"],
        vec!["fmt", "--stdin", "--check", "--write"],
        vec!["fmt", "--stdin", "--check", "--check"],
    ] {
        let output = cli().args(arguments).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

/// Atomic replacement retains file permission bits and declines symbolic-link replacement.
#[cfg(unix)]
#[test]
fn formatting_write_preserves_permissions_and_rejects_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (scratch, path) = fixture(SOURCE);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let link = scratch.path().join("alias.holo");
    symlink(&path, &link).unwrap();
    let output = cli().arg("fmt").arg(&link).arg("--write").output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    success(cli().arg("fmt").arg(&path).arg("--write").output().unwrap());
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    // Even an already-canonical symlink has no regular-file replacement contract.
    let output = cli().arg("fmt").arg(&link).arg("--write").output().unwrap();
    assert!(!output.status.success());
}

/// All new help paths are available without experimental flags or source acquisition.
#[test]
fn help_documents_text_tools_without_native_dependencies() {
    let main = success(cli().arg("--help").output().unwrap());
    assert!(main.contains("highlight"));
    assert!(main.contains("fmt"));
    assert!(success(cli().args(["highlight", "--help"]).output().unwrap()).contains("--style"));
    assert!(success(cli().args(["fmt", "--help"]).output().unwrap()).contains("--write"));
}
