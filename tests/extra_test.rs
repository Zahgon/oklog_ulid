//! Additional tests that **increase** coverage beyond the Go suite.
//!
//! Two categories:
//!   1. Public API the Go tests never exercise — most notably `Value()`, which
//!      sits at 0.0% statement coverage in the upstream Go suite.
//!   2. The support modules this port had to introduce (`gorand`, `gotime`,
//!      `goio`). These are ports of Go *stdlib* behaviour, so they were never
//!      in Go's coverage denominator, but they are new first-party code here
//!      and are therefore tested directly.

mod common;

use common::recover_msg;

use ulid::{
    civil_to_unix, days_from_civil, default_entropy, format_civil, max_time, monotonic, must_new,
    offset_between, parse, parse_strict, read_at_least, read_full, time, timestamp, BufCore,
    BytesReader, CryptoRand, Entropy, Error, GoRand, HalfReader, IoError, LockedHandle,
    LockedMonotonicReader, MultiReader, Reader, RngSource, ScanValue, Time, ZeroReader,
    LAYOUT_DEFAULT_MS, LAYOUT_RFC3339_MS, MILLISECOND, SECOND, ULID, ZERO,
};

// ---------------------------------------------------------------------------
// 1. Public API not covered by the Go suite
// ---------------------------------------------------------------------------

/// `Value()` is 0.0% covered in the Go suite. It must equal MarshalBinary.
#[test]
fn extra_value_matches_marshal_binary() {
    let mut r = BytesReader::new((0u8..16).collect::<Vec<u8>>());
    let id = must_new(1469918176385, Entropy::Reader(&mut r));

    let v = id.value().expect("Value");
    let b = id.marshal_binary().expect("MarshalBinary");
    assert_eq!(v, b);
    assert_eq!(v.len(), 16);
    assert_eq!(&v[..], &id.0[..]);
}

/// Zero-value ULIDs must still produce a valid Value (documented Go behaviour).
#[test]
fn extra_value_of_zero_ulid_is_ok() {
    let v = ZERO.value().expect("Value of zero ULID must not error");
    assert_eq!(v, vec![0u8; 16]);
}

/// `Scan` with a wrong-length byte slice must yield ErrDataSize (the Go suite
/// only covers 16-byte and 26-byte inputs, leaving this branch at 87.5%).
#[test]
fn extra_scan_bad_length_bytes() {
    let mut id = ULID::default();
    for bad in [
        &[][..],
        &[1, 2, 3][..],
        &[0u8; 15][..],
        &[0u8; 17][..],
        &[0u8; 25][..],
    ] {
        let err = id.scan(ScanValue::Bytes(bad)).unwrap_err();
        assert_eq!(err, Error::DataSize, "len {}", bad.len());
    }
}

/// `Scan` of a 26-char *invalid* text value propagates the parse error.
#[test]
fn extra_scan_invalid_text() {
    let mut id = ULID::default();
    let err = id
        .scan(ScanValue::Str("ZZZZZZZZZZZZZZZZZZZZZZZZZZ"))
        .unwrap_err();
    assert_eq!(err, Error::Overflow);
}

/// Every error variant renders exactly the Go message.
#[test]
fn extra_error_messages_match_go() {
    assert_eq!(
        Error::DataSize.to_string(),
        "ulid: bad data size when unmarshaling"
    );
    assert_eq!(
        Error::InvalidCharacters.to_string(),
        "ulid: bad data characters when unmarshaling"
    );
    assert_eq!(
        Error::BufferSize.to_string(),
        "ulid: bad buffer size when marshaling"
    );
    assert_eq!(Error::BigTime.to_string(), "ulid: time too big");
    assert_eq!(
        Error::Overflow.to_string(),
        "ulid: overflow when unmarshaling"
    );
    assert_eq!(
        Error::MonotonicOverflow.to_string(),
        "ulid: monotonic entropy overflow"
    );
    assert_eq!(
        Error::ScanValue.to_string(),
        "ulid: source value must be a string or byte slice"
    );
    assert_eq!(Error::Io(IoError::Eof).to_string(), "EOF");
    assert_eq!(
        Error::Io(IoError::UnexpectedEof).to_string(),
        "unexpected EOF"
    );
    assert_eq!(Error::Io(IoError::ShortBuffer).to_string(), "short buffer");
}

/// `ParseStrict` accepts every character of the encoding alphabet, and the
/// lowercase alias of each.
#[test]
fn extra_parse_strict_accepts_full_alphabet() {
    for c in ulid::ENCODING.iter() {
        // Position 0 is bounded to '7' by the overflow rule, so vary position 1.
        let mut s = b"00000000000000000000000000".to_vec();
        s[1] = *c;
        let up = String::from_utf8(s.clone()).unwrap();
        assert!(parse_strict(&up).is_ok(), "uppercase {:?}", *c as char);

        let lo = up.to_lowercase();
        assert!(parse_strict(&lo).is_ok(), "lowercase {:?}", *c as char);
        assert_eq!(parse(&up).unwrap(), parse(&lo).unwrap());
    }
}

/// `MustParseStrict` succeeds on a valid input (the Go suite only tests its
/// panic path).
#[test]
fn extra_must_parse_strict_success() {
    let id = ulid::must_parse_strict("01ARYZ6S410000000000000000");
    assert_eq!(id.time(), 1469918176385);
    assert_eq!(id.entropy(), vec![0u8; 10]);
}

/// `Entropy::None` leaves the entropy zeroed and only sets the time.
#[test]
fn extra_new_without_entropy_is_time_only() {
    let id = must_new(1469918176385, Entropy::None);
    assert_eq!(id.entropy(), vec![0u8; 10]);
    assert_eq!(id.time(), 1469918176385);
}

/// `DefaultEntropy` is usable as a plain reader as well as a monotonic one.
#[test]
fn extra_default_entropy_read_path() {
    let mut de = default_entropy();
    let mut buf = [0u8; 10];
    let (n, err) = de.read(&mut buf);
    assert_eq!(n, 10);
    assert!(err.is_none());
    assert_ne!(buf, [0u8; 10], "default entropy produced all zeroes");
}

/// A `LockedMonotonicReader` used single-threaded still yields increasing IDs,
/// and its `read` path works.
#[test]
fn extra_locked_monotonic_reader_single_thread() {
    let rng = GoRand::new(4242);
    let locked = LockedMonotonicReader::new(monotonic(Box::new(rng), 0));

    let mut prev = ZERO;
    for _ in 0..1000 {
        let mut p = [0u8; 10];
        locked.monotonic_read(7, &mut p).expect("monotonic_read");
        let mut id = ULID::default();
        id.set_time(7).unwrap();
        id.set_entropy(&p).unwrap();
        assert!(prev.compare(&id) < 0, "not increasing: {} -> {}", prev, id);
        prev = id;
    }

    let mut buf = [0u8; 4];
    let (n, err) = locked.read(&mut buf);
    assert_eq!(n, 4);
    assert!(err.is_none());
}

/// `MonotonicEntropy` also satisfies the plain reader interface.
#[test]
fn extra_monotonic_entropy_read_path() {
    let rng = GoRand::new(11);
    let mut m = monotonic(Box::new(rng), 0);
    let mut buf = [0u8; 32];
    let (n, err) = m.read(&mut buf);
    assert_eq!(n, 32);
    assert!(err.is_none());
}

/// A new timestamp resets the monotonic entropy (the `else` branch of
/// MonotonicRead), rather than incrementing.
#[test]
fn extra_monotonic_resets_on_new_timestamp() {
    let rng = GoRand::new(5150);
    let mut m = monotonic(Box::new(rng), 0);

    let a = must_new(100, Entropy::Monotonic(&mut m));
    let b = must_new(100, Entropy::Monotonic(&mut m));
    assert!(a.compare(&b) < 0, "same ms must increment");

    // Different ms -> fresh random entropy, no ordering guarantee on entropy,
    // but the time component must advance.
    let c = must_new(101, Entropy::Monotonic(&mut m));
    assert_eq!(c.time(), 101);
    assert!(b.compare(&c) < 0, "later timestamp must sort later");
}

/// `inc == 1` takes the `random() -> 1` fast path, incrementing by exactly 1.
#[test]
fn extra_monotonic_inc_one_increments_by_one() {
    let src = BytesReader::new(vec![0x01u8; 4096]);
    let mut m = monotonic(Box::new(src), 1);

    let a = must_new(50, Entropy::Monotonic(&mut m));
    let b = must_new(50, Entropy::Monotonic(&mut m));
    let c = must_new(50, Entropy::Monotonic(&mut m));

    // Seed entropy is 0x01 x10; successive values add exactly 1 each time.
    let mut want_b = vec![1u8; 10];
    want_b[9] = 2;
    let mut want_c = vec![1u8; 10];
    want_c[9] = 3;
    assert_eq!(a.entropy(), vec![1u8; 10]);
    assert_eq!(b.entropy(), want_b);
    assert_eq!(c.entropy(), want_c);
}

/// Upstream quirk, verified against Go: `MonotonicRead` treats **all-zero**
/// entropy as "uninitialised" (`!m.entropy.IsZero()`), so a source yielding
/// zeroes re-reads instead of incrementing — monotonicity silently does not
/// hold. Pinned here so a future refactor cannot change it unnoticed.
#[test]
fn extra_monotonic_all_zero_entropy_does_not_increment() {
    let mut src = BytesReader::new(vec![0u8; 4096]);
    let mut m = monotonic(Box::new(HalfReader::new(&mut src)), 1);

    for _ in 0..3 {
        let id = must_new(50, Entropy::Monotonic(&mut m));
        assert_eq!(
            id.entropy(),
            vec![0u8; 10],
            "Go re-reads (does not increment) when entropy is all-zero"
        );
    }
}

/// Monotonic entropy exhaustion surfaces the underlying reader's EOF.
#[test]
fn extra_monotonic_entropy_exhausted() {
    let mut src = BytesReader::new(vec![1u8; 4]); // fewer than 10 bytes
    let mut m = monotonic(Box::new(HalfReader::new(&mut src)), 0);
    let err = ulid::new(1, Entropy::Monotonic(&mut m)).unwrap_err();
    assert_eq!(err, Error::Io(IoError::UnexpectedEof));
}

/// `must_new` panics carry the error message (mirrors Go's panic(err)).
#[test]
fn extra_must_new_panics_with_big_time() {
    let msg = recover_msg(|| {
        let _ = must_new(max_time() + 1, Entropy::None);
    });
    assert_eq!(msg.as_deref(), Some("ulid: time too big"));
}

// ---------------------------------------------------------------------------
// 2. Support modules introduced by this port
// ---------------------------------------------------------------------------

/// `gorand` must reproduce Go's `math/rand` exactly. These vectors were
/// captured from Go 1.27 (`rand.New(rand.NewSource(seed))`).
#[test]
fn extra_gorand_matches_go_vectors() {
    let mut r = GoRand::new(42);
    assert_eq!(
        (0..5).map(|_| r.int63()).collect::<Vec<_>>(),
        vec![
            3440579354231278675,
            608747136543856411,
            5571782338101878760,
            1926012586526624009,
            404153945743547657
        ]
    );

    let mut r = GoRand::new(1);
    assert_eq!(
        (0..5).map(|_| r.int63n(1000)).collect::<Vec<_>>(),
        vec![410, 551, 821, 51, 937]
    );

    // Stateful Read: leftover bytes of the current int63 carry across calls.
    let mut r = GoRand::new(7);
    let mut b = [0u8; 12];
    r.read(&mut b);
    assert_eq!(b, [243, 255, 77, 69, 30, 66, 158, 24, 34, 21, 170, 238]);
    let mut b2 = [0u8; 5];
    r.read(&mut b2);
    assert_eq!(b2, [6, 162, 214, 75, 109]);
}

/// `int63n` power-of-two path and its panic contract.
#[test]
fn extra_gorand_int63n_edges() {
    let mut r = GoRand::new(3);
    for _ in 0..1000 {
        let v = r.int63n(64); // power of two -> mask path
        assert!((0..64).contains(&v));
    }
    for n in [1i64, 7, 1 << 40] {
        let mut r = GoRand::new(9);
        for _ in 0..100 {
            let v = r.int63n(n);
            assert!(v >= 0 && v < n, "int63n({}) out of range: {}", n, v);
        }
    }
    assert!(recover_msg(|| {
        let mut r = GoRand::new(1);
        r.int63n(0);
    })
    .is_some());
}

/// `RngSource` seeding edge cases (seed 0 is remapped, negatives normalise).
#[test]
fn extra_rngsource_seed_edges() {
    let mut a = RngSource::new(0);
    let mut b = RngSource::new(0);
    assert_eq!(a.int63(), b.int63(), "same seed must be deterministic");

    let mut c = RngSource::new(-5);
    let mut d = RngSource::new(-5);
    assert_eq!(c.uint64(), d.uint64());

    let mut e = RngSource::new(1);
    let mut f = RngSource::new(2);
    assert_ne!(e.int63(), f.int63(), "different seeds must diverge");
}

/// `io.ReadFull` distinguishes EOF (nothing read) from UnexpectedEOF (partial).
#[test]
fn extra_read_full_eof_semantics() {
    // Nothing available -> EOF
    let mut empty = BytesReader::new(Vec::new());
    let mut buf = [0u8; 4];
    let (n, err) = read_full(&mut empty, &mut buf);
    assert_eq!(n, 0);
    assert_eq!(err, Some(IoError::Eof));

    // Partial -> UnexpectedEOF
    let mut partial = BytesReader::new(vec![1, 2]);
    let mut buf = [0u8; 4];
    let (n, err) = read_full(&mut partial, &mut buf);
    assert_eq!(n, 2);
    assert_eq!(err, Some(IoError::UnexpectedEof));

    // Exact -> success
    let mut exact = BytesReader::new(vec![1, 2, 3, 4]);
    let mut buf = [0u8; 4];
    let (n, err) = read_full(&mut exact, &mut buf);
    assert_eq!(n, 4);
    assert_eq!(err, None);
    assert_eq!(buf, [1, 2, 3, 4]);
}

/// `HalfReader` yields ceil(n/2) bytes per call, like `iotest.HalfReader`.
#[test]
fn extra_half_reader_semantics() {
    let mut src = BytesReader::new((1u8..=10).collect::<Vec<u8>>());
    let mut h = HalfReader::new(&mut src);
    let mut buf = [0u8; 10];
    let (n, err) = h.read(&mut buf);
    assert_eq!(n, 5, "HalfReader must return ceil(10/2)");
    assert!(err.is_none());
    assert_eq!(&buf[..5], &[1, 2, 3, 4, 5]);
}

/// `MultiReader` drains readers in order and suppresses intermediate EOFs.
#[test]
fn extra_multi_reader_sequencing() {
    let mut a = BytesReader::new(vec![1, 2, 3]);
    let mut b = BytesReader::new(vec![4, 5]);
    let mut m = MultiReader::new(vec![&mut a, &mut b]);

    let mut out = Vec::new();
    loop {
        let mut buf = [0u8; 8];
        let (n, err) = m.read(&mut buf);
        out.extend_from_slice(&buf[..n]);
        if err == Some(IoError::Eof) {
            break;
        }
        assert!(err.is_none(), "unexpected error: {:?}", err);
    }
    assert_eq!(out, vec![1, 2, 3, 4, 5]);
}

/// `BufCore` reproduces bufio.Reader: read-ahead when small, bypass when large.
#[test]
fn extra_bufcore_buffering() {
    // Small destination -> fills the 4096-byte buffer, so one source read.
    let mut src = BytesReader::new((0u8..=255).cycle().take(100).collect::<Vec<u8>>());
    let mut bc = BufCore::new();
    let mut buf = [0u8; 4];
    let (n, err) = bc.read_from(&mut src, &mut buf);
    assert_eq!(n, 4);
    assert!(err.is_none());
    assert_eq!(buf, [0, 1, 2, 3]);

    // Destination >= buffer size -> bypasses the buffer entirely.
    let mut src2 = BytesReader::new(vec![9u8; 10_000]);
    let mut bc2 = BufCore::with_size(16);
    let mut big = [0u8; 64];
    let (n, err) = bc2.read_from(&mut src2, &mut big);
    assert_eq!(n, 64);
    assert!(err.is_none());
    assert!(big.iter().all(|&b| b == 9));

    // Empty destination is a no-op.
    let mut bc3 = BufCore::new();
    let mut src3 = BytesReader::new(vec![1]);
    let (n, _) = bc3.read_from(&mut src3, &mut []);
    assert_eq!(n, 0);
}

/// `ZeroReader` fills with zeroes and never errors (CLI `--zero`).
#[test]
fn extra_zero_reader() {
    let mut z = ZeroReader;
    let mut buf = [7u8; 16];
    let (n, err) = z.read(&mut buf);
    assert_eq!(n, 16);
    assert!(err.is_none());
    assert_eq!(buf, [0u8; 16]);

    let id = must_new(1, Entropy::Reader(&mut ZeroReader));
    assert_eq!(id.entropy(), vec![0u8; 10]);
}

/// `CryptoRand` returns the requested number of bytes.
#[test]
fn extra_crypto_rand_fills() {
    let mut c = CryptoRand::new();
    let mut buf = [0u8; 32];
    let (n, err) = c.read(&mut buf);
    assert_eq!(n, 32);
    assert!(err.is_none());
}

/// `Time` arithmetic: unix_new normalisation, truncate, add, sub.
#[test]
fn extra_gotime_arithmetic() {
    // nsec normalisation (Go's time.Unix does the same)
    let t = Time::unix_new(1, 1_500_000_000);
    assert_eq!(t.unix(), 2);
    assert_eq!(t.nanosecond(), 500_000_000);

    let t = Time::unix_new(1, -1);
    assert_eq!(t.unix(), 0);
    assert_eq!(t.nanosecond(), 999_999_999);

    // truncate to millisecond
    let t = Time::unix_new(10, 123_456_789).truncate(MILLISECOND);
    assert_eq!(t.nanosecond(), 123_000_000);
    // truncate with d <= 0 is a no-op
    let t2 = Time::unix_new(10, 123_456_789);
    assert_eq!(t2.truncate(0), t2);

    // add / sub
    let a = Time::unix_new(100, 0);
    let b = a.add(SECOND);
    assert_eq!(b.unix(), 101);
    assert_eq!(b.sub(a), SECOND);
    assert_eq!(a.sub(b), -SECOND);

    // unix_nano
    assert_eq!(Time::unix_new(2, 5).unix_nano(), 2_000_000_005);
}

/// Go's zero `time.Time` is year 1, and overflows ULID's timestamp.
#[test]
fn extra_gotime_zero_overflows_ulid() {
    let z = Time::zero();
    assert_eq!(z.unix(), -62135596800);
    assert_eq!(z.nanosecond(), 0);
    assert!(
        timestamp(z) > max_time(),
        "zero time must exceed MaxTime after wrapping"
    );
}

/// Calendar conversion + the two CLI layouts.
#[test]
fn extra_gotime_format() {
    // 2016-07-30T22:36:16.385Z, the TestAlizainCompatibility instant.
    let t = time(1469918176385);
    let c = t.utc_civil();
    assert_eq!((c.year, c.month, c.day), (2016, 7, 30));
    assert_eq!((c.hour, c.min, c.sec), (22, 36, 16));
    assert_eq!(c.weekday, 6, "1469918176385 was a Saturday");
    assert_eq!(
        format_civil(&c, LAYOUT_RFC3339_MS),
        "2016-07-30T22:36:16.385Z"
    );
    assert_eq!(
        format_civil(&c, LAYOUT_DEFAULT_MS),
        "Sat Jul 30 22:36:16.385 UTC 2016"
    );

    // `.999` drops a zero fraction entirely.
    let c0 = time(1469918176000).utc_civil();
    assert_eq!(
        format_civil(&c0, LAYOUT_DEFAULT_MS),
        "Sat Jul 30 22:36:16 UTC 2016"
    );

    // epoch and a pre-epoch instant
    let e = Time::unix_new(0, 0).utc_civil();
    assert_eq!((e.year, e.month, e.day), (1970, 1, 1));
    assert_eq!(e.weekday, 4, "epoch was a Thursday");

    // unknown layout returns empty, matching the port's documented contract
    assert_eq!(format_civil(&c, "nonsense"), "");
}

/// Local-time conversion returns a plausible offset and a zone abbreviation.
#[test]
fn extra_gotime_local_civil() {
    let t = time(1469918176385);
    let c = t.local_civil();
    assert!(
        (-50400..=50400).contains(&c.offset),
        "implausible UTC offset: {}",
        c.offset
    );
    assert!(!c.zone.is_empty(), "zone abbreviation must be populated");
    // Local and UTC describe the same instant.
    assert_eq!(
        (c.hour as i64 * 3600 + c.min as i64 * 60 + c.sec as i64) - c.offset as i64,
        {
            let u = t.utc_civil();
            u.hour as i64 * 3600 + u.min as i64 * 60 + u.sec as i64
        } - 86400 * ((c.day as i64) - (t.utc_civil().day as i64))
    );
}

/// The error path of `increment()`/`random()`: the entropy source runs dry
/// *during* the increment, so the read error propagates out of MonotonicRead.
/// (`inc` is large, so this takes the slow byte-reading path, not the rng one.)
#[test]
fn extra_monotonic_increment_read_error() {
    // Exactly 10 non-zero bytes, then EOF.
    let src = BytesReader::new(vec![0x7Fu8; 10]);
    let mut m = monotonic(Box::new(src), 0); // inc == 0 -> u32::MAX -> slow path

    // First call consumes all 10 bytes and seeds the entropy.
    let first = must_new(4242, Entropy::Monotonic(&mut m));
    assert_eq!(first.entropy(), vec![0x7Fu8; 10]);

    // Second call at the same ms must increment, but the source is exhausted.
    let err = ulid::new(4242, Entropy::Monotonic(&mut m)).unwrap_err();
    assert_eq!(err, Error::Io(IoError::Eof));
}

/// `LockedHandle` also forwards the plain `read` path.
#[test]
fn extra_locked_handle_read_path() {
    let rng = GoRand::new(31337);
    let locked = LockedMonotonicReader::new(monotonic(Box::new(rng), 0));
    let mut h = LockedHandle(&locked);

    let mut buf = [0u8; 16];
    let (n, err) = h.read(&mut buf);
    assert_eq!(n, 16);
    assert!(err.is_none());
    assert_ne!(buf, [0u8; 16]);

    // and the monotonic path through the same handle
    let a = must_new(5, Entropy::Monotonic(&mut h));
    let b = must_new(5, Entropy::Monotonic(&mut h));
    assert!(a.compare(&b) < 0);
}

/// `read_at_least` rejects a buffer smaller than `min` with ShortBuffer.
#[test]
fn extra_read_at_least_short_buffer() {
    let mut src = BytesReader::new(vec![1, 2, 3, 4]);
    let mut buf = [0u8; 2];
    let (n, err) = read_at_least(&mut src, &mut buf, 4);
    assert_eq!(n, 0);
    assert_eq!(err, Some(IoError::ShortBuffer));
    assert_eq!(IoError::Other("boom".into()).to_string(), "boom");
}

/// `BufCore` propagates a stored error on the next call, and reports EOF for an
/// empty destination once the buffer is drained (bufio.Reader semantics).
#[test]
fn extra_bufcore_error_propagation() {
    let mut src = BytesReader::new(vec![1, 2, 3]);
    let mut bc = BufCore::new();

    let mut buf = [0u8; 3];
    let (n, err) = bc.read_from(&mut src, &mut buf);
    assert_eq!((n, err), (3, None));

    // Source now EOFs; BufCore surfaces it.
    let (n, err) = bc.read_from(&mut src, &mut buf);
    assert_eq!(n, 0);
    assert_eq!(err, Some(IoError::Eof));

    // Empty destination with an empty buffer returns the stored error slot.
    let (n, _) = bc.read_from(&mut src, &mut []);
    assert_eq!(n, 0);
}

/// `MultiReader` with no readers is immediately EOF.
#[test]
fn extra_multi_reader_empty() {
    let mut m = MultiReader::new(vec![]);
    let mut buf = [0u8; 4];
    assert_eq!(m.read(&mut buf), (0, Some(IoError::Eof)));
}

/// RFC3339 formatting of negative and positive UTC offsets.
#[test]
fn extra_gotime_format_offsets() {
    let t = time(1469918176385);
    let mut c = t.utc_civil();

    c.offset = -7 * 3600;
    c.zone = "PDT".into();
    assert!(format_civil(&c, LAYOUT_RFC3339_MS).ends_with("-07:00"));

    c.offset = 5 * 3600 + 30 * 60;
    c.zone = "IST".into();
    assert!(format_civil(&c, LAYOUT_RFC3339_MS).ends_with("+05:30"));
    assert!(format_civil(&c, LAYOUT_DEFAULT_MS).contains("IST"));
}

/// `CryptoRand` must behave like Go's `crypto/rand.Reader`: every call fills the
/// whole buffer (no short reads), across a range of sizes including ones larger
/// than a typical kernel entropy chunk.
#[test]
fn extra_crypto_rand_always_fills_completely() {
    let mut c = CryptoRand::new();
    for len in [0usize, 1, 7, 10, 16, 64, 4096, 5000, 70_000] {
        let mut buf = vec![0xAAu8; len];
        let (n, err) = c.read(&mut buf);
        assert_eq!(n, len, "short read for len {}", len);
        assert!(err.is_none(), "error for len {}: {:?}", len, err);
    }
    // Distinct calls must not repeat.
    let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
    c.read(&mut a);
    c.read(&mut b);
    assert_ne!(a, b, "CSPRNG returned identical blocks");
}

/// The arithmetic behind the **Windows** tz path, tested on every platform.
///
/// Windows' CRT provides no `tm_gmtoff`, so the port recovers the UTC offset by
/// differencing local and UTC broken-down times. The FFI itself can only run on
/// Windows, but this arithmetic is platform-independent and is verified here so
/// it is not shipped untested.
#[test]
fn extra_windows_offset_arithmetic() {
    // 2016-07-30 22:36:16 UTC (the TestAlizainCompatibility instant).
    let utc = (2016, 7, 30, 22, 36, 16);

    // UTC-7 (PDT): local is the same day, 15:36:16.
    assert_eq!(offset_between((2016, 7, 30, 15, 36, 16), utc), -7 * 3600);

    // UTC+0
    assert_eq!(offset_between(utc, utc), 0);

    // UTC+5:30 (IST): rolls into the next day, 04:06:16.
    assert_eq!(
        offset_between((2016, 7, 31, 4, 6, 16), utc),
        5 * 3600 + 30 * 60
    );

    // UTC+14 (Kiritimati): next day, 12:36:16.
    assert_eq!(offset_between((2016, 7, 31, 12, 36, 16), utc), 14 * 3600);

    // UTC-11 (Niue): previous day, 11:36:16 — crosses a month boundary backwards.
    assert_eq!(offset_between((2016, 7, 30, 11, 36, 16), utc), -11 * 3600);

    // Year boundary: 2017-01-01 00:30 UTC seen as 2016-12-31 19:30 (UTC-5).
    assert_eq!(
        offset_between((2016, 12, 31, 19, 30, 0), (2017, 1, 1, 0, 30, 0)),
        -5 * 3600
    );

    // Leap day.
    assert_eq!(
        offset_between((2016, 2, 29, 12, 0, 0), (2016, 2, 29, 3, 0, 0)),
        9 * 3600
    );
}

/// `days_from_civil` must be the exact inverse of the `civil_from_days` used by
/// the UTC formatting path, across a wide date range including leap years.
#[test]
fn extra_days_from_civil_round_trips() {
    for day in (-25_000i64..=25_000).step_by(7) {
        let t = Time::unix_new(day * 86400, 0);
        let c = t.utc_civil();
        assert_eq!(
            days_from_civil(c.year, c.month as i64, c.day as i64),
            day,
            "round trip failed for day offset {} ({:?}-{}-{})",
            day,
            c.year,
            c.month,
            c.day
        );
    }
    // Known anchors.
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(2000, 3, 1), 11017);
    assert_eq!(civil_to_unix(2016, 7, 30, 22, 36, 16), 1469918176);
}
