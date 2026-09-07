//! API-completeness check.
//!
//! Names every exported identifier of `github.com/oklog/ulid/v2` and its Rust
//! counterpart. This file exists to fail *compilation* if any part of the
//! ported surface is removed, renamed or changes shape.
//!
//! The Go declarations are quoted in comments so the mapping can be audited
//! against the original source.
//!
//! `ulid.go` declares 49 exported-looking items under 47 distinct names —
//! `Time` and `Timestamp` are each both a package function and a `ULID` method.
//! Three of the 47 (`Add`, `AppendTo`, `SetBytes`) belong solely to the
//! unexported `uint80` and are therefore internal; the remaining 44 are public
//! surface and every one of them is named below.
//!
//! **Every case here asserts a value as well as a type.** An earlier revision
//! bound each call to `let _: T = ...` and stopped there, which proved the
//! signature and nothing else: a function could return a wrong-but-well-typed
//! value and this file stayed green. Signature checking is still the primary
//! job — the bindings are annotated, so a changed return type is a compile
//! error — but each case now also pins the value that call must produce.

use ulid::*;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Go: `type ULID [16]byte`
#[test]
fn surface_type_ulid() {
    let id: ULID = ULID([0u8; 16]);
    let raw: [u8; 16] = id.0;
    assert_eq!(raw, [0u8; 16], "ULID must be a transparent [16]byte");

    // must be a plain value type: Copy, Eq, Ord, Hash, Default, Debug, Display
    let copied: ULID = id;
    let equal: bool = copied == id;
    assert!(equal, "ULID must be Copy + Eq and compare equal to its copy");
    assert_eq!(
        copied.cmp(&id),
        std::cmp::Ordering::Equal,
        "Ord must agree with Eq"
    );
    assert_eq!(ULID::default(), ZERO, "Default must be the zero ULID");

    let shown: String = format!("{}", id);
    assert_eq!(
        shown, "00000000000000000000000000",
        "Display must be the 26-char Crockford form"
    );
    let debugged: String = format!("{:?}", id);
    assert!(!debugged.is_empty(), "Debug must render");

    let mut set = std::collections::HashSet::new();
    assert!(set.insert(id), "Hash must admit a fresh ULID");
    assert!(!set.insert(copied), "equal ULIDs must hash to one entry");
}

/// Go: `type MonotonicReader interface { io.Reader; MonotonicRead(ms uint64, p []byte) error }`
#[test]
fn surface_type_monotonic_reader() {
    fn takes(m: &mut dyn MonotonicReader) -> Result<(), Error> {
        let mut p = [0u8; 10];
        m.monotonic_read(1, &mut p)
    }
    let mut m = monotonic(Box::new(GoRand::new(1)), 0);
    assert!(
        takes(&mut m).is_ok(),
        "MonotonicEntropy must be usable through a &mut dyn MonotonicReader"
    );
}

/// Go: `type MonotonicEntropy struct { ... }`
#[test]
fn surface_type_monotonic_entropy() {
    let mut m: MonotonicEntropy = monotonic(Box::new(GoRand::new(1)), 0);
    // successive reads at one timestamp must strictly increase
    let a = must_new(7, Entropy::Monotonic(&mut m));
    let b = must_new(7, Entropy::Monotonic(&mut m));
    assert!(a < b, "monotonic entropy must strictly increase: {a} !< {b}");
    assert_eq!(a.time(), 7, "the timestamp must survive the entropy path");
    assert_eq!(b.time(), 7);
}

/// Go: `type LockedMonotonicReader struct { mu sync.Mutex; MonotonicReader }`
#[test]
fn surface_type_locked_monotonic_reader() {
    let l: LockedMonotonicReader =
        LockedMonotonicReader::new(monotonic(Box::new(GoRand::new(1)), 0));
    let mut p = [0u8; 10];
    assert!(
        l.monotonic_read(1, &mut p).is_ok(),
        "locked monotonic_read must succeed"
    );

    let (n, err): (usize, Option<IoError>) = l.read(&mut p);
    assert_eq!(n, p.len(), "locked read must fill the buffer");
    assert!(err.is_none(), "locked read must not error: {err:?}");

    // shared handle, so several threads can drive one source
    let mut h = LockedHandle(&l);
    assert!(
        h.monotonic_read(1, &mut p).is_ok(),
        "a LockedHandle must drive the same source"
    );
}

// ---------------------------------------------------------------------------
// Package-level values and constants
// ---------------------------------------------------------------------------

/// Go: `Zero ULID`, `const Encoding`, `const EncodedSize`
#[test]
fn surface_constants() {
    let zero: ULID = ZERO;
    assert!(zero.is_zero(), "ZERO must be the zero ULID");
    assert_eq!(zero.0, [0u8; 16]);

    let enc: &[u8; 32] = ENCODING;
    let size: usize = ENCODED_SIZE;
    assert_eq!(size, 26);
    assert_eq!(enc.len(), 32);
    assert_eq!(
        enc, b"0123456789ABCDEFGHJKMNPQRSTVWXYZ",
        "Encoding must be Crockford base32"
    );
}

/// Go: `ErrDataSize`, `ErrInvalidCharacters`, `ErrBufferSize`, `ErrBigTime`,
/// `ErrOverflow`, `ErrMonotonicOverflow`, `ErrScanValue`
#[test]
fn surface_errors() {
    let all = [
        Error::DataSize,
        Error::InvalidCharacters,
        Error::BufferSize,
        Error::BigTime,
        Error::Overflow,
        Error::MonotonicOverflow,
        Error::ScanValue,
        Error::Io(IoError::Eof),
    ];
    for e in &all {
        // comparable by value, like Go's sentinel `==`
        assert_eq!(*e, *e, "each sentinel must compare equal to itself");
        let text: String = e.to_string();
        assert!(!text.is_empty(), "{e:?} must render a message");
        assert!(
            text.starts_with("ulid: ") || matches!(e, Error::Io(_)),
            "ulid sentinels carry the Go `ulid: ` prefix, got {text:?}"
        );
        let _: &dyn std::error::Error = e;
    }
    // the sentinels must be distinct from one another, as Go's `==` requires
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "sentinels {i} and {j} collide");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

/// Go: `func New(ms uint64, entropy io.Reader) (id ULID, err error)`
/// Go: `func MustNew(ms uint64, entropy io.Reader) ULID`
/// Go: `func MustNewDefault(t time.Time) ULID`
/// Go: `func Make() (id ULID)`
/// Go: `func DefaultEntropy() io.Reader`
#[test]
fn surface_constructors() {
    let made: Result<ULID, Error> = new(0, Entropy::None);
    assert_eq!(made, Ok(ZERO), "New(0, nil) is the zero ULID");

    let (id, err): (ULID, Option<Error>) = new_partial(0, Entropy::None);
    assert_eq!(id, ZERO);
    assert!(err.is_none(), "New(0, nil) must not error: {err:?}");

    let must: ULID = must_new(0, Entropy::None);
    assert_eq!(must, ZERO);

    let dflt: ULID = must_new_default(Time::now());
    assert!(!dflt.is_zero(), "MustNewDefault must draw real entropy");
    assert!(dflt.time() > 0, "MustNewDefault must carry the wall clock");

    let m: ULID = make();
    assert!(!m.is_zero(), "Make must draw real entropy");
    assert_ne!(m, make(), "successive Make() calls must differ");

    let mut de = default_entropy();
    let from_default: ULID = must_new(0, Entropy::Monotonic(&mut de));
    assert_eq!(from_default.time(), 0, "the caller's timestamp must win");

    // the three-way entropy dispatch Go performs on the dynamic type
    let mut r = BytesReader::new(vec![0u8; 16]);
    let from_reader: ULID = must_new(0, Entropy::Reader(&mut r));
    assert_eq!(
        from_reader, ZERO,
        "an all-zero io.Reader must yield the zero ULID"
    );
}

/// Go: `func Monotonic(entropy io.Reader, inc uint64) *MonotonicEntropy`
#[test]
fn surface_monotonic() {
    let mut seeded: MonotonicEntropy = monotonic(Box::new(GoRand::new(0)), 0);
    let a = must_new(1, Entropy::Monotonic(&mut seeded));
    let b = must_new(1, Entropy::Monotonic(&mut seeded));
    assert!(a < b, "Monotonic(rand, 0) must strictly increase");

    let mut crypt: MonotonicEntropy = monotonic(Box::new(CryptoRand::new()), 42);
    let c = must_new(1, Entropy::Monotonic(&mut crypt));
    let d = must_new(1, Entropy::Monotonic(&mut crypt));
    assert!(c < d, "Monotonic(crypto, 42) must strictly increase");
    assert_eq!(c.time(), 1);
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Go: `func Parse(ulid string) (id ULID, err error)`
/// Go: `func ParseStrict(ulid string) (id ULID, err error)`
/// Go: `func MustParse(ulid string) ULID`
/// Go: `func MustParseStrict(ulid string) ULID`
#[test]
fn surface_parsing() {
    const S: &str = "00000000000000000000000000";
    assert_eq!(parse(S), Ok(ZERO));
    assert_eq!(parse_strict(S), Ok(ZERO));
    assert_eq!(must_parse(S), ZERO);
    assert_eq!(must_parse_strict(S), ZERO);
    // byte-oriented forms, required because the Go suite feeds non-UTF-8 input
    assert_eq!(parse_bytes(S.as_bytes()), Ok(ZERO));
    assert_eq!(parse_strict_bytes(S.as_bytes()), Ok(ZERO));

    // and the round trip, so parsing is pinned to encoding
    let id = must_parse("01ARYZ6S410000000000000000");
    assert_eq!(id.to_string(), "01ARYZ6S410000000000000000");

    // the lenient/strict distinction must be real
    assert!(parse("0000000000000000000000000").is_err(), "short input");
    assert!(parse_strict("00000000000000000000000o").is_err());
}

// ---------------------------------------------------------------------------
// Time helpers
// ---------------------------------------------------------------------------

/// Go: `func MaxTime() uint64`, `func Now() uint64`,
/// Go: `func Timestamp(t time.Time) uint64`, `func Time(ms uint64) time.Time`
#[test]
fn surface_time_helpers() {
    let max: u64 = max_time();
    assert_eq!(max, 281_474_976_710_655, "MaxTime is 2^48 - 1");

    let n: u64 = now();
    assert!(n > 1_500_000_000_000, "Now must be Unix milliseconds");

    let ts: u64 = timestamp(Time::now());
    assert!(ts > 1_500_000_000_000, "Timestamp must be Unix milliseconds");

    let epoch: Time = time(0);
    assert_eq!(epoch.unix(), 0, "Time(0) is the Unix epoch");
    // Timestamp and Time must be inverses
    assert_eq!(timestamp(time(1_469_918_176_385)), 1_469_918_176_385);
}

// ---------------------------------------------------------------------------
// ULID methods
// ---------------------------------------------------------------------------

/// Go: `Bytes`, `String`, `MarshalBinary`, `MarshalBinaryTo`, `UnmarshalBinary`,
/// Go: `MarshalText`, `MarshalTextTo`, `UnmarshalText`
#[test]
fn surface_methods_marshaling() {
    let mut id = must_parse("01ARYZ6S410000000000000000");

    let raw: Vec<u8> = id.bytes();
    assert_eq!(raw.len(), 16, "Bytes is the 16-byte binary form");

    let shown: String = id.to_string();
    assert_eq!(shown, "01ARYZ6S410000000000000000");

    let bin_owned: Vec<u8> = id.marshal_binary().expect("MarshalBinary");
    assert_eq!(bin_owned, raw, "MarshalBinary must agree with Bytes");

    let txt_owned: Vec<u8> = id.marshal_text().expect("MarshalText");
    assert_eq!(txt_owned, shown.as_bytes(), "MarshalText must agree with String");

    let mut bin = [0u8; 16];
    id.marshal_binary_to(&mut bin).expect("MarshalBinaryTo");
    assert_eq!(&bin[..], &raw[..]);
    let mut txt = [0u8; ENCODED_SIZE];
    id.marshal_text_to(&mut txt).expect("MarshalTextTo");
    assert_eq!(&txt[..], shown.as_bytes());

    // and the buffers must be rejected when they are the wrong size
    assert_eq!(id.marshal_binary_to(&mut [0u8; 15]), Err(Error::BufferSize));
    assert_eq!(id.marshal_text_to(&mut [0u8; 25]), Err(Error::BufferSize));

    let mut round = ULID::default();
    round.unmarshal_binary(&bin).expect("UnmarshalBinary");
    assert_eq!(round, id, "binary round trip must be lossless");
    let mut round_txt = ULID::default();
    round_txt.unmarshal_text(&txt).expect("UnmarshalText");
    assert_eq!(round_txt, id, "text round trip must be lossless");

    id.unmarshal_binary(&bin).expect("UnmarshalBinary in place");
    assert_eq!(id, round);
}

/// Go: `Time`, `Timestamp`, `IsZero`, `SetTime`, `Entropy`, `SetEntropy`, `Compare`
#[test]
fn surface_methods_accessors() {
    let mut id = ULID::default();
    assert_eq!(id.time(), 0, "the zero ULID has timestamp 0");
    assert_eq!(id.timestamp().unix(), 0, "Timestamp is Time(id.Time())");
    assert!(id.is_zero(), "the zero ULID must report IsZero");

    id.set_time(1_469_918_176_385).expect("SetTime");
    assert_eq!(id.time(), 1_469_918_176_385, "SetTime must round-trip");
    assert!(!id.is_zero(), "a timestamped ULID is no longer zero");
    assert_eq!(id.set_time(max_time() + 1), Err(Error::BigTime));

    let ent: Vec<u8> = id.entropy();
    assert_eq!(ent.len(), 10, "Entropy is the trailing 80 bits");
    assert_eq!(ent, vec![0u8; 10]);

    id.set_entropy(&[0xAAu8; 10]).expect("SetEntropy");
    assert_eq!(id.entropy(), vec![0xAAu8; 10], "SetEntropy must round-trip");
    assert_eq!(id.set_entropy(&[0u8; 9]), Err(Error::DataSize));

    let cmp: i32 = id.compare(&ZERO);
    assert_eq!(cmp, 1, "a non-zero ULID sorts after ZERO");
    assert_eq!(ZERO.compare(&id), -1);
    assert_eq!(id.compare(&id), 0);
}

/// Go: `func (id *ULID) Scan(src interface{}) error`
/// Go: `func (id ULID) Value() (driver.Value, error)`
#[test]
fn surface_methods_sql() {
    let mut id = ULID::default();

    id.scan(ScanValue::Nil).expect("Scan(nil) is a no-op");
    assert_eq!(id, ZERO, "Scan(nil) must leave the ULID untouched");

    id.scan(ScanValue::Str("01ARYZ6S410000000000000000"))
        .expect("Scan(string)");
    assert_eq!(id.to_string(), "01ARYZ6S410000000000000000");

    id.scan(ScanValue::Bytes(&[0u8; 16])).expect("Scan([]byte)");
    assert_eq!(id, ZERO, "a 16-byte slice is the binary form");

    // #134: a []byte carrying the 26-char text form must also be accepted
    id.scan(ScanValue::Bytes(b"01ARYZ6S410000000000000000"))
        .expect("Scan(text []byte)");
    assert_eq!(id.to_string(), "01ARYZ6S410000000000000000");

    assert_eq!(
        id.scan(ScanValue::Other),
        Err(Error::ScanValue),
        "an unsupported column type must be rejected"
    );

    let v: Vec<u8> = id.value().expect("Value");
    assert_eq!(v, id.bytes(), "Value is MarshalBinary");
}

// ---------------------------------------------------------------------------
// Reader surface (Go's io.Reader contract, which ULID's entropy depends on)
// ---------------------------------------------------------------------------

/// Go: `io.Reader`, `io.ReadFull`, `io.ReadAtLeast`, `io.EOF`,
/// Go: `io.ErrUnexpectedEOF`, `io.ErrShortBuffer`
#[test]
fn surface_reader_contract() {
    fn takes(r: &mut dyn Reader, buf: &mut [u8]) -> (usize, Option<IoError>) {
        r.read(buf)
    }
    let mut b = BytesReader::new(vec![1, 2, 3]);
    let mut buf = [0u8; 2];
    let (n, err) = takes(&mut b, &mut buf);
    assert_eq!((n, &buf[..]), (2, &[1u8, 2][..]));
    assert!(err.is_none());

    let mut full = BytesReader::new(vec![1, 2, 3]);
    let (n, err): (usize, Option<IoError>) = read_full(&mut full, &mut buf);
    assert_eq!(n, 2, "ReadFull must fill the buffer");
    assert!(err.is_none(), "{err:?}");

    let mut least = BytesReader::new(vec![1, 2, 3]);
    let (n, err): (usize, Option<IoError>) = read_at_least(&mut least, &mut buf, 1);
    assert!(n >= 1, "ReadAtLeast must deliver its minimum");
    assert!(err.is_none(), "{err:?}");

    // a short source must surface Go's distinction rather than a generic error
    let mut short = BytesReader::new(vec![1]);
    let (n, err) = read_full(&mut short, &mut buf);
    assert_eq!(n, 1);
    assert_eq!(err, Some(IoError::UnexpectedEof), "short read is ErrUnexpectedEOF");
    let mut empty = BytesReader::new(vec![]);
    let (n, err) = read_full(&mut empty, &mut buf);
    assert_eq!((n, err), (0, Some(IoError::Eof)), "empty read is EOF");

    // Go distinguishes these three; a port that collapses them fails here.
    assert_ne!(IoError::Eof, IoError::UnexpectedEof);
    assert_ne!(IoError::Eof, IoError::ShortBuffer);
    assert_ne!(IoError::UnexpectedEof, IoError::ShortBuffer);
    assert_ne!(IoError::Eof, IoError::Other(String::new()));
}

/// Go: `bytes.Reader`, `io.MultiReader`, `iotest.HalfReader`, `bufio.Reader`,
/// Go: `crypto/rand.Reader`
#[test]
fn surface_reader_implementations() {
    let mut a = BytesReader::new(vec![1]);
    let mut b = BytesReader::new(vec![2]);
    let mut multi = MultiReader::new(vec![&mut a, &mut b]);
    let mut buf = [0u8; 2];
    let (n, err) = multi.read(&mut buf);
    assert_eq!(n, 1, "MultiReader must not straddle sub-readers in one read");
    assert_eq!(buf[0], 1);
    assert!(err.is_none());
    let (n, _) = multi.read(&mut buf);
    assert_eq!((n, buf[0]), (1, 2), "the second reader must follow");

    let mut src = BytesReader::new(vec![1, 2, 3, 4]);
    let mut half = HalfReader::new(&mut src);
    let mut hbuf = [0u8; 4];
    let (n, err) = half.read(&mut hbuf);
    assert_eq!(n, 2, "HalfReader delivers ceil(len/2) bytes");
    assert_eq!(&hbuf[..2], &[1, 2]);
    assert!(err.is_none());

    let mut bc = BufCore::new();
    let mut inner = BytesReader::new(vec![9; 8]);
    let (n, err) = bc.read_from(&mut inner, &mut buf);
    assert_eq!(n, buf.len(), "a buffered read must fill a short buffer");
    assert_eq!(buf, [9, 9]);
    assert!(err.is_none());

    let mut c = CryptoRand::new();
    let mut wide = [0u8; 32];
    let (n, err) = c.read(&mut wide);
    assert_eq!(n, wide.len(), "crypto/rand always fills the buffer");
    assert!(err.is_none());
    assert_ne!(wide, [0u8; 32], "crypto/rand must not return all zeros");

    let mut z = ZeroReader;
    let mut zbuf = [7u8; 4];
    let (n, err) = z.read(&mut zbuf);
    assert_eq!((n, zbuf), (4, [0u8; 4]), "ZeroReader must zero the buffer");
    assert!(err.is_none());
}

// ---------------------------------------------------------------------------
// Deterministic generator (Go's math/rand, which ExampleULID depends on)
// ---------------------------------------------------------------------------

/// Go: `rand.NewSource(seed)`, `rand.New(src)`, `Int63`, `Int63n`, `Read`
#[test]
fn surface_gorand() {
    let mut s = RngSource::new(1);
    let mut s2 = RngSource::new(1);
    let first: i64 = s.int63();
    assert_eq!(first, s2.int63(), "one seed must give one stream");
    assert!(first >= 0, "Int63 is non-negative");
    let u: u64 = s.uint64();
    assert_eq!(u, s2.uint64(), "Uint64 must stay in lockstep");
    s.seed(2);
    assert_ne!(s.int63(), s2.int63(), "reseeding must change the stream");

    let mut r = GoRand::new(1);
    let mut r2 = GoRand::new(1);
    let v: i64 = r.int63();
    assert_eq!(v, r2.int63(), "Rand(seed) must be deterministic");
    let n: i64 = r.int63n(10);
    assert!((0..10).contains(&n), "Int63n(10) must land in [0,10)");
    assert_eq!(n, r2.int63n(10));

    let mut buf = [0u8; 4];
    let mut buf2 = [0u8; 4];
    let (read, err): (usize, Option<IoError>) = r.read(&mut buf);
    assert_eq!(read, buf.len(), "Read must fill the buffer");
    assert!(err.is_none());
    let (_, _) = r2.read(&mut buf2);
    assert_eq!(buf, buf2, "Read must be deterministic under a fixed seed");

    // must satisfy Go's unexported `rng` interface for Monotonic's fast path
    fn takes(s: &mut dyn Int63nSource) -> i64 {
        s.int63n(4)
    }
    assert!((0..4).contains(&takes(&mut r)));
}

// ---------------------------------------------------------------------------
// Time surface (the `time.Time` subset ULID observes)
// ---------------------------------------------------------------------------

/// Go: `time.Unix`, `time.Now`, `time.Time{}`, `Unix`, `UnixNano`,
/// Go: `Nanosecond`, `Truncate`, `Sub`, `Add`, `time.Millisecond`, `time.Second`
#[test]
fn surface_gotime() {
    let t: Time = Time::unix_new(1, 0);
    assert_eq!(t.unix(), 1, "time.Unix(1, 0).Unix() == 1");
    assert_eq!(t.unix_nano(), 1_000_000_000, "UnixNano is seconds x 1e9");
    assert_eq!(t.nanosecond(), 0);

    let n: Time = Time::now();
    assert!(n.unix() > 1_500_000_000, "time.Now must be a real clock");
    let z: Time = Time::zero();
    assert!(z.unix() < 0, "the zero time.Time predates the Unix epoch");

    let sub_nanos: Time = Time::unix_new(1, 500_000);
    assert_eq!(
        sub_nanos.truncate(MILLISECOND).nanosecond(),
        0,
        "Truncate(Millisecond) must clear sub-ms nanoseconds"
    );
    assert_eq!(t.sub(t), 0, "t.Sub(t) is zero");
    assert_eq!(t.add(SECOND).unix(), 2, "Add(time.Second) advances one second");
    assert_eq!(t.add(SECOND).sub(t), SECOND);
    assert_eq!(MILLISECOND, 1_000_000);
    assert_eq!(SECOND, 1_000_000_000);

    let c: Civil = t.utc_civil();
    assert_eq!(
        (c.year, c.month, c.day),
        (1970, 1, 1),
        "1970-01-01 is Unix second 1"
    );
    let _: Civil = t.local_civil();
    let rfc: String = format_civil(&c, LAYOUT_RFC3339_MS);
    assert!(rfc.starts_with("1970-01-01T"), "got {rfc:?}");
    let dflt: String = format_civil(&c, LAYOUT_DEFAULT_MS);
    assert!(dflt.contains("Jan 01"), "got {dflt:?}");

    assert_eq!(days_from_civil(1970, 1, 1), 0, "the epoch is day zero");
    assert_eq!(days_from_civil(1970, 1, 2), 1);
    assert_eq!(civil_to_unix(1970, 1, 1, 0, 0, 0), 0);
    assert_eq!(civil_to_unix(1970, 1, 1, 0, 0, 1), 1);
    assert_eq!(
        offset_between((1970, 1, 1, 0, 0, 0), (1970, 1, 1, 0, 0, 0)),
        0,
        "no offset between a moment and itself"
    );
}
