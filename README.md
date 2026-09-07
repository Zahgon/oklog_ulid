# ulid-rs

A 1:1 Rust port of [`github.com/oklog/ulid/v2`](https://github.com/oklog/ulid) (Go).

Every exported Go identifier has a Rust equivalent. The bit-level encoding, error
values and messages, overflow rules, monotonic-entropy behaviour and CLI surface
are reproduced exactly.

```
cargo test                      # 178 tests
cargo run --example bench       # ported Benchmark* functions
cargo run --bin ulid -- --help  # ported cmd/ulid
```

## Verification

| Check | Result |
|---|---|
| Ported tests (1:1 with Go's leaf cases) | **103 / 103 pass** |
| Added tests (`extra`, `cli`, `api_surface`, `parity`) | **75 pass** |
| Total | **178 pass**, 0 failed, 0 ignored (macOS arm64, Linux arm64, Linux x86_64) |
| Parity vs Go — corpus (`tests/parity.rs`) | **63,945 / 63,945 lines byte-identical** |
| Parity vs Go — exhaustive parse sweep | **6,656 / 6,656 cases identical** |
| `ExampleULID` exact output | `0000XSNJG0MQJHBF4QX1EFD6Y3` ✓ |
| Go `math/rand` stream (`Int63`, `Int63n`, `Read`) | bit-exact ✓ |
| CLI vs Go binary (4 formats, `--local`, error paths, exit codes) | identical ✓ |
| Mutation testing (10 injected defects) | **10 / 10 caught** |
| `cargo clippy --all-targets` | 0 warnings |
| Platforms compile-checked | macOS arm64, Linux x86_64 + arm64, Windows MSVC |

### Coverage vs the Go original

| Component | Go | Rust |
|---|---|---|
| Library (`ulid.go` → `lib.rs`) | 98.6% | **99.48%** |
| CLI (`cmd/ulid` → `bin/ulid.rs`) | 0.0% | **88.89%** |
| Total | 83.1% | **96.39%** |

The parity suite (`tests/parity.rs`, replaying `tests/testdata/*.txt` generated
by the equivalent Go program) compares 20,000
ULIDs across `String`/`Time`/`Entropy`/`Compare`/`IsZero`/`Timestamp`,
text+binary marshaling, case-folded round trips, 208 parse-mutation error cases,
all 9 `Scan`/`Value` dynamic types, partial-read entropy via `HalfReader`
(including the `EOF` vs `UnexpectedEOF` distinction), 300 `LockedMonotonicReader`
IDs, 3,400 monotonic-sequence IDs across every `inc` byte-length bucket, and the
timestamp boundaries, and every byte value at every position for both parse
modes. The goldens are Go's output, regenerated identically on a second
platform — a specification, not a snapshot of this crate.

## API mapping

| Go | Rust |
|---|---|
| `ULID [16]byte` | `ULID(pub [u8; 16])` |
| `New(ms, entropy)` | `new(ms, Entropy) -> Result<ULID, Error>` |
| — (Go's `(id, err)` tuple) | `new_partial(ms, Entropy) -> (ULID, Option<Error>)` |
| `MustNew`, `MustNewDefault`, `Make` | `must_new`, `must_new_default`, `make` |
| `Parse`, `ParseStrict` | `parse`, `parse_strict` (+ `parse_bytes`, `parse_strict_bytes`) |
| `MustParse`, `MustParseStrict` | `must_parse`, `must_parse_strict` |
| `id.String()` | `impl Display for ULID` |
| `MarshalBinary/Text(+To)` | `marshal_binary/text(_to)` |
| `UnmarshalBinary/Text` | `unmarshal_binary/text` |
| `Time`, `Timestamp`, `SetTime` | `time`, `timestamp`, `set_time` |
| `Entropy`, `SetEntropy`, `Compare`, `IsZero`, `Bytes` | `entropy`, `set_entropy`, `compare`, `is_zero`, `bytes` |
| `Scan(interface{})` / `Value()` | `scan(ScanValue)` / `value()` |
| `Monotonic`, `MonotonicEntropy` | `monotonic`, `MonotonicEntropy` |
| `LockedMonotonicReader` | `LockedMonotonicReader` (+ `LockedHandle` for sharing) |
| `DefaultEntropy()`, `MaxTime()`, `Now()` | `default_entropy()`, `max_time()`, `now()` |
| `Encoding`, `EncodedSize`, `Zero` | `ENCODING`, `ENCODED_SIZE`, `ZERO` |
| `ErrDataSize`, `ErrOverflow`, … | `Error::{DataSize, Overflow, …}` (identical `Display` text) |

## Notable porting decisions

**Go's `math/rand` was ported bit-for-bit** (`src/gorand.rs` + the 607-entry
`rngCooked` table). This is not optional: `ExampleULID` asserts an exact output
string from a fixed seed, and several tests seed deterministically. The port
reproduces `rngSource.Seed/Uint64/Int63`, `Int63n`, and the stateful
`readVal`/`readPos` buffering of `Rand.Read`.

**Go's `io.Reader` contract is modelled directly** rather than reusing
`std::io::Read`. Go returns `(n, err)` and distinguishes `io.EOF` from
`io.ErrUnexpectedEOF` in `io.ReadFull` — observable in `TestNew` (empty reader
yields `EOF`, not `UnexpectedEOF`) and `TestEntropyRead` (`iotest.HalfReader`).

**`bufio.Reader` read-ahead is preserved.** `Monotonic` wraps its source in a
`bufio.Reader` while *also* holding the raw source for the `Int63n` fast path —
an alias Rust's borrow checker forbids. Resolved by having `MonotonicEntropy`
own the source and hold the buffer state (`BufCore`) alongside it, passing the
source per call. Byte-consumption order is therefore unchanged.

**`entropy io.Reader` became an explicit `Entropy` enum.** Go dispatches on the
dynamic type (`nil` / `MonotonicReader` / plain reader); the enum makes that
three-way dispatch explicit and total.

**Overflow arithmetic is explicitly wrapping.** `Timestamp` uses
`wrapping_mul`/`wrapping_add` so Go's zero `time.Time` (year 1, Unix second
`-62135596800`) still overflows into `ErrBigTime` — which
`TestMustNewDefault/Panic` depends on.

**`time.Time` is reduced to an instant** (`sec` + `nsec`), which is all any ULID
path observes. The CLI's `--local` reads the platform tz database via a
dependency-free `extern "C" localtime_r`, giving both the UTC offset and the
zone abbreviation Go's `MST` layout element prints.

## Known differences

These are the deliberate deviations. Behaviour of every ported code path is
identical; these are API-shape differences.

- **`Reader` has a `Send` supertrait**, which Go's `io.Reader` does not. Needed
  so the process-wide `DefaultEntropy` can live in a `static`. It slightly
  restricts what callers may implement.
- `Value()` returns `Vec<u8>` instead of `driver.Value`, and `Scan` takes a
  `ScanValue` enum instead of `interface{}`; Rust has no `database/sql`.
- `Bytes()` returns `Vec<u8>` (Go returns a `[]byte` copy from a value
  receiver); `TestULID_Bytes` — which asserts non-aliasing — passes.
- `gotime::Time` is a reduced `time.Time` (an instant, plus the two layouts the
  CLI needs). It is not a general-purpose date library; consumers wanting one
  should convert to `chrono`/`time`.
- The `quick` helper is seeded deterministically rather than from the clock, so
  failures reproduce. Iteration counts match Go exactly.
- Benchmarks are a plain example harness, since `testing.B` has no stable std
  equivalent.
- The library has **zero third-party dependencies**, matching upstream (Go's
  only dependency, `pborman/getopt`, was used by the CLI and is reimplemented).

## Platform support

| Platform | Entropy source | tz lookup (`--local`) | Verification |
|---|---|---|---|
| macOS arm64 | `/dev/urandom` (full-fill, `EINTR`-safe) | `localtime_r` | **runtime**: 158 tests + both differentials |
| Linux arm64 | same | same | **runtime** (Docker): 158 tests + both differentials |
| Linux x86_64 | same | same | **runtime** (Docker): 158 tests + both differentials |
| Windows MSVC | `BCryptGenRandom` | `_localtime64_s`/`_gmtime64_s` + `_get_tzname` | **compile-checked**; offset arithmetic unit-tested on all platforms. Raw FFI calls not executed. |
| Other | error from reader | falls back to UTC | — |

The Go reference output was itself confirmed identical on macOS and Linux, so
the differential corpus is platform-stable.

On Windows the zone *abbreviation* comes from the CRT (`tzname`), which may
render differently from Go's tzdata-derived abbreviation. Only the CLI's
`--local` display is affected; the UTC offset and every library path are
unaffected.
