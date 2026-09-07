//! End-to-end tests for the `ulid` binary (port of `cmd/ulid/main.go`).
//!
//! The upstream Go CLI has **no tests at all** (0.0% statement coverage), so
//! this suite is a net coverage and behaviour improvement. Expected values were
//! captured from the Go binary built from the original repository.

use std::process::Command;

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_ulid"))
        .args(args)
        .output()
        .expect("failed to run ulid binary");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// A fixed ULID whose timestamp is 2016-07-30T22:36:16.385Z.
const ID: &str = "01ARYZ6S410000000000000000";

/// `--format default` matches Go's `Mon Jan 02 15:04:05.999 MST 2006` layout.
/// NOTE: the Go original writes parse output to stderr; that is preserved.
#[test]
fn cli_parse_default_format() {
    let (_, err, code) = run(&["-f", "default", ID]);
    assert_eq!(code, 0);
    assert_eq!(err.trim(), "Sat Jul 30 22:36:16.385 UTC 2016");
}

#[test]
fn cli_parse_rfc3339_format() {
    let (_, err, code) = run(&["-f", "rfc3339", ID]);
    assert_eq!(code, 0);
    assert_eq!(err.trim(), "2016-07-30T22:36:16.385Z");
}

#[test]
fn cli_parse_unix_format() {
    let (_, err, code) = run(&["-f", "unix", ID]);
    assert_eq!(code, 0);
    assert_eq!(err.trim(), "1469918176");
}

#[test]
fn cli_parse_ms_format() {
    let (_, err, code) = run(&["-f", "ms", ID]);
    assert_eq!(code, 0);
    assert_eq!(err.trim(), "1469918176385");
}

/// The long form and `--format=value` spelling behave identically.
#[test]
fn cli_long_and_equals_flag_forms() {
    let (_, a, _) = run(&["--format", "unix", ID]);
    let (_, b, _) = run(&["--format=unix", ID]);
    let (_, c, _) = run(&["-funix", ID]);
    assert_eq!(a.trim(), "1469918176");
    assert_eq!(a, b);
    assert_eq!(a, c);
}

/// Format matching is case-insensitive (Go lowercases before switching).
#[test]
fn cli_format_is_case_insensitive() {
    let (_, err, code) = run(&["-f", "RFC3339", ID]);
    assert_eq!(code, 0);
    assert_eq!(err.trim(), "2016-07-30T22:36:16.385Z");
}

/// `--local` renders the same instant with a zone abbreviation and offset.
#[test]
fn cli_local_flag() {
    let (_, utc, _) = run(&["-f", "rfc3339", ID]);
    let (_, local, code) = run(&["-l", "-f", "rfc3339", ID]);
    assert_eq!(code, 0);
    let local = local.trim();
    assert!(
        local.ends_with('Z') || local.contains('+') || local.matches('-').count() >= 3,
        "expected an offset or Z in {:?}",
        local
    );
    // Same instant, so the ms component is unchanged.
    assert!(local.contains(".385"), "{:?}", local);
    assert!(utc.trim().contains(".385"));
}

/// An invalid ULID exits 1 with the library's error message.
#[test]
fn cli_invalid_ulid_errors() {
    let (_, err, code) = run(&["not-a-ulid"]);
    assert_eq!(code, 1);
    assert_eq!(err.trim(), "ulid: bad data size when unmarshaling");
}

/// An overflowing ULID surfaces ErrOverflow.
#[test]
fn cli_overflow_ulid_errors() {
    let (_, err, code) = run(&["ZZZZZZZZZZZZZZZZZZZZZZZZZZ"]);
    assert_eq!(code, 1);
    assert_eq!(err.trim(), "ulid: overflow when unmarshaling");
}

/// An unknown `--format` exits 1 before doing any work.
#[test]
fn cli_invalid_format_errors() {
    let (_, err, code) = run(&["-f", "nope", ID]);
    assert_eq!(code, 1);
    assert_eq!(err.trim(), "invalid --format nope");
}

/// `--help` prints usage to stderr and exits 0.
#[test]
fn cli_help_exits_zero() {
    let (_, err, code) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(err.contains("--format"), "usage text missing: {:?}", err);
    assert!(err.contains("--quick"));
    assert!(err.contains("--zero"));
}

/// The usage block is byte-for-byte what `fs.PrintUsage` emits in the Go
/// original. `contains` checks above cannot catch a line-wrap divergence, so
/// this pins the exact text.
#[test]
fn cli_help_matches_go_byte_for_byte() {
    // `getopt.New()` names the option set after `path.Base(os.Args[0])`, so the
    // first line tracks whatever the executable is called. Under `cargo test`
    // that is `ulid`; the invariant is that it is the binary's own basename,
    // which `cli_help_usage_line_follows_argv0` pins independently.
    // Captured from the Go binary. Every option row is indented by exactly one
    // space; `concat!` is used rather than a `"\` continuation because that
    // escape eats the leading whitespace of the following line, which is how
    // the port lost this space without any test noticing.
    const GO_USAGE_BODY: &str = concat!(
        " -f, --format=<format>  when parsing, show times in this format: default, rfc3339, unix, ms\n",
        " -h, --help             print this help text\n",
        " -l, --local            when parsing, show local time instead of UTC\n",
        " -q, --quick            when generating, use non-crypto-grade entropy\n",
        " -z, --zero             when generating, fix entropy to all-zeroes\n",
    );
    let expected = format!(
        "Usage: {} [-hlqz] [-f <format>] [parameters ...]\n{}",
        std::path::Path::new(env!("CARGO_BIN_EXE_ulid"))
            .file_name()
            .unwrap()
            .to_string_lossy(),
        GO_USAGE_BODY
    );
    let (out, err, code) = run(&["--help"]);
    assert_eq!(code, 0);
    assert_eq!(out, "", "usage must not go to stdout");
    assert_eq!(err, expected);
}

/// The usage line must name the executable, exactly as Go's getopt does.
///
/// The Go original derives the name from `os.Args[0]`; an earlier revision of
/// this port hardcoded `ulid`, so copying or installing the binary under any
/// other name silently diverged from the original. No test caught it, because
/// under `cargo test` the binary is in fact called `ulid`.
#[test]
fn cli_help_usage_line_follows_argv0() {
    let dir = std::env::temp_dir().join(format!("ulid-argv0-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let renamed = dir.join("ulidtool");
    std::fs::copy(env!("CARGO_BIN_EXE_ulid"), &renamed).expect("copy binary");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&renamed, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }

    let out = Command::new(&renamed)
        .arg("--help")
        .output()
        .expect("run renamed binary");
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(out.status.code(), Some(0));
    assert!(
        err.starts_with("Usage: ulidtool [-hlqz] [-f <format>] [parameters ...]\n"),
        "usage must name the executable, got: {:?}",
        err.lines().next()
    );
}

/// With no arguments the CLI generates a fresh, parseable ULID on stdout.
#[test]
fn cli_generate_default() {
    let (out, _, code) = run(&[]);
    assert_eq!(code, 0);
    let s = out.trim();
    assert_eq!(s.len(), 26, "got {:?}", s);
    assert!(ulid::parse_strict(s).is_ok(), "not a valid ULID: {:?}", s);
}

/// `--zero` fixes the entropy to all zeroes; only the time component varies.
#[test]
fn cli_generate_zero_entropy() {
    let (out, _, code) = run(&["-z"]);
    assert_eq!(code, 0);
    let s = out.trim();
    let id = ulid::parse_strict(s).expect("valid ULID");
    assert_eq!(id.entropy(), vec![0u8; 10]);
    assert_eq!(&s[10..], "0000000000000000");
}

/// `--quick` uses the non-crypto generator but still yields a valid ULID.
#[test]
fn cli_generate_quick_entropy() {
    let (out, _, code) = run(&["-q"]);
    assert_eq!(code, 0);
    assert!(ulid::parse_strict(out.trim()).is_ok());
}

/// Successive generations are distinct and time-ordered.
#[test]
fn cli_generate_is_unique() {
    let (a, _, _) = run(&[]);
    let (b, _, _) = run(&[]);
    assert_ne!(a.trim(), b.trim());
    let (ia, ib) = (
        ulid::parse(a.trim()).unwrap(),
        ulid::parse(b.trim()).unwrap(),
    );
    assert!(ia.time() <= ib.time());
}

/// Combined short flags (`-zl`) are accepted, like getopt.
#[test]
fn cli_combined_short_flags() {
    let (out, _, code) = run(&["-zq"]);
    assert_eq!(code, 0);
    // -z wins over -q, matching the Go ordering (zero checked last).
    let id = ulid::parse_strict(out.trim()).expect("valid ULID");
    assert_eq!(id.entropy(), vec![0u8; 10]);
}

/// A missing value for `-f` is an error.
#[test]
fn cli_missing_format_value() {
    let (_, err, code) = run(&["-f"]);
    assert_eq!(code, 1);
    assert!(err.contains("missing parameter"), "{:?}", err);
}
