//! Port of `cmd/ulid/main.go`.
//!
//! The Go original uses `github.com/pborman/getopt/v2`; this reimplements the
//! same flag surface (long + short forms, `--flag=value` and `--flag value`)
//! without pulling in a dependency.

use std::io::Write;
use std::process::exit;

use ulid::goio::{CryptoRand, Reader, ZeroReader};
use ulid::gorand::GoRand;
use ulid::gotime::{self, Time, LAYOUT_DEFAULT_MS, LAYOUT_RFC3339_MS};
use ulid::{Entropy, ULID};

/// Everything after the `Usage:` line, which is fixed text.
///
/// Written with `concat!` rather than a `"\` continuation on purpose: a
/// backslash-newline in a Rust string literal swallows the newline *and the
/// leading whitespace of the next line*, which silently deleted the single
/// leading space getopt puts in front of every option row.
const USAGE_BODY: &str = concat!(
    " -f, --format=<format>  when parsing, show times in this format: default, rfc3339, unix, ms\n",
    " -h, --help             print this help text\n",
    " -l, --local            when parsing, show local time instead of UTC\n",
    " -q, --quick            when generating, use non-crypto-grade entropy\n",
    " -z, --zero             when generating, fix entropy to all-zeroes\n",
);

/// `getopt.New()` names the option set after `path.Base(os.Args[0])`, and
/// `PrintUsage` prints that name — so renaming or copying the Go binary changes
/// its usage line. Hardcoding `ulid` here made the port diverge from the
/// original the moment the executable was installed under any other name.
fn program_name() -> String {
    std::env::args_os()
        .next()
        .map(|a| {
            std::path::Path::new(&a)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| a.to_string_lossy().into_owned())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| String::from("ulid"))
}

fn usage() -> String {
    format!(
        "Usage: {} [-hlqz] [-f <format>] [parameters ...]\n{}",
        program_name(),
        USAGE_BODY
    )
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    let mut format = String::from("default");
    let mut local = false;
    let mut quick = false;
    let mut zero = false;
    let mut help = false;
    let mut args: Vec<String> = Vec::new();

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].clone();
        match a.as_str() {
            "--" => {
                args.extend_from_slice(&argv[i + 1..]);
                break;
            }
            "-l" | "--local" => local = true,
            "-q" | "--quick" => quick = true,
            "-z" | "--zero" => zero = true,
            "-h" | "--help" => help = true,
            "-f" | "--format" => {
                i += 1;
                if i >= argv.len() {
                    eprintln!("missing parameter for -f");
                    exit(1);
                }
                format = argv[i].clone();
            }
            _ if a.starts_with("--format=") => {
                format = a["--format=".len()..].to_string();
            }
            _ if a.starts_with("-f") && a.len() > 2 => {
                format = a[2..].to_string();
            }
            // Combined short flags, e.g. `-lq`.
            _ if a.starts_with('-') && a.len() > 1 && !a.starts_with("--") => {
                for c in a[1..].chars() {
                    match c {
                        'l' => local = true,
                        'q' => quick = true,
                        'z' => zero = true,
                        'h' => help = true,
                        _ => {
                            eprintln!("unknown option: -{}", c);
                            exit(1);
                        }
                    }
                }
            }
            _ if a.starts_with("--") => {
                eprintln!("unknown option: {}", a);
                exit(1);
            }
            _ => args.push(a),
        }
        i += 1;
    }

    if help {
        eprint!("{}", usage());
        exit(0);
    }

    // Selects the formatter; an unknown format is rejected before any work.
    let format_kind = match format.to_lowercase().as_str() {
        "default" => FormatKind::Default,
        "rfc3339" => FormatKind::Rfc3339,
        "unix" => FormatKind::Unix,
        "ms" => FormatKind::Ms,
        _ => {
            eprintln!("invalid --format {}", format);
            exit(1);
        }
    };

    if args.is_empty() {
        generate(quick, zero);
    } else {
        parse_one(&args[0], local, format_kind);
    }
}

#[derive(Clone, Copy)]
enum FormatKind {
    Default,
    Rfc3339,
    Unix,
    Ms,
}

fn format_time(t: Time, local: bool, kind: FormatKind) -> String {
    match kind {
        FormatKind::Unix => format!("{}", t.unix()),
        FormatKind::Ms => format!("{}", t.unix_nano() / 1_000_000),
        FormatKind::Default => {
            let c = if local {
                t.local_civil()
            } else {
                t.utc_civil()
            };
            gotime::format_civil(&c, LAYOUT_DEFAULT_MS)
        }
        FormatKind::Rfc3339 => {
            let c = if local {
                t.local_civil()
            } else {
                t.utc_civil()
            };
            gotime::format_civil(&c, LAYOUT_RFC3339_MS)
        }
    }
}

fn generate(quick: bool, zero: bool) {
    let mut crypto = CryptoRand::new();
    let mut math = GoRand::new(Time::now().unix_nano());
    let mut zeroes = ZeroReader;

    let entropy: &mut dyn Reader = if zero {
        &mut zeroes
    } else if quick {
        &mut math
    } else {
        &mut crypto
    };

    match ulid::new(ulid::timestamp(Time::now()), Entropy::Reader(entropy)) {
        Ok(id) => {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{}", id);
        }
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    }
}

fn parse_one(s: &str, local: bool, kind: FormatKind) {
    let id: ULID = match ulid::parse(s) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    };

    let t = ulid::time(id.time());
    // NOTE: the Go original writes the parsed time to stderr; preserved here.
    eprintln!("{}", format_time(t, local, kind));
}
