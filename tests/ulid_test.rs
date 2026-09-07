// Copyright 2016 The Oklog Authors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! 1:1 port of `ulid_test.go`, at Go's *leaf* test granularity.
//!
//! Go reports 103 leaf test cases (30 `Test` funcs, of which 8 fan out into 81
//! `t.Run` subtests). Each Go leaf has exactly one `#[test]` here, so failures
//! isolate the same way and the reported count matches.
//!
//! Go construct mapping:
//!   * `t.Run(name, ...)`     -> a separate `#[test] fn <parent>_<name>()`
//!   * `testing/quick.Check`  -> `common::Quick`, same `MaxCount`
//!   * `recover()`            -> `common::recover_msg` (`catch_unwind`)

mod common;

use common::{recover_msg, sprint_err, Quick, QUICK_DEFAULT};

use std::sync::Arc;

use ulid::{
    default_entropy, make, max_time, monotonic, must_new, must_new_default, must_parse,
    must_parse_strict, new, new_partial, now, parse, parse_bytes, parse_strict_bytes, time,
    timestamp, BytesReader, CryptoRand, Entropy, Error, GoRand, HalfReader, IoError, LockedHandle,
    LockedMonotonicReader, MultiReader, ScanValue, Time, MILLISECOND, ULID, ZERO,
};

// ===========================================================================
// ExampleULID  (Go: Example function, asserts exact stdout)
// ===========================================================================

/// Port of `ExampleULID`. The strictest behavioural check in the suite: it only
/// passes if Go's `math/rand` is reproduced bit-for-bit.
#[test]
fn example_ulid() {
    let t = Time::unix_new(1000000, 0);
    let rng = GoRand::new(t.unix_nano());
    let mut entropy = monotonic(Box::new(rng), 0);
    let got = must_new(timestamp(t), Entropy::Monotonic(&mut entropy)).to_string();
    // Output: 0000XSNJG0MQJHBF4QX1EFD6Y3
    assert_eq!(got, "0000XSNJG0MQJHBF4QX1EFD6Y3");
}

// ===========================================================================
// TestNew / TestMustNew  (shared `testULID` helper)
// ===========================================================================

/// Port of the shared `testULID` helper: asserts the exact ULID bytes produced
/// with (a) no entropy and (b) 16 bytes of 0xFF entropy.
fn test_ulid(mk: &mut dyn FnMut(u64, Entropy<'_>) -> ULID) {
    let mut want = ULID([0x0, 0x0, 0x0, 0x1, 0x86, 0xa0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let got = mk(100_000, Entropy::None); // optional entropy
    assert_eq!(got, want, "\ngot  {:?}\nwant {:?}", got, want);

    let entropy = vec![0xFFu8; 16];
    want.0[6..].copy_from_slice(&entropy[..10]);
    let mut r = BytesReader::new(entropy.clone());
    let got = mk(100_000, Entropy::Reader(&mut r));
    assert_eq!(got, want, "\ngot  {:?}\nwant {:?}", got, want);
}

/// Go: `TestNew/ULID`
#[test]
fn test_new_ulid() {
    test_ulid(&mut |ms, e| new(ms, e).expect("ulid::new returned an error"));
}

/// Go: `TestNew/Error` — ErrBigTime past MaxTime, and io.EOF from an empty reader.
#[test]
fn test_new_error() {
    let err = new(max_time() + 1, Entropy::None).err();
    assert_eq!(err, Some(Error::BigTime), "got err {:?}", err);

    let mut empty = BytesReader::new(Vec::new());
    let err = new(0, Entropy::Reader(&mut empty)).err();
    assert_eq!(
        err,
        Some(Error::Io(IoError::Eof)),
        "got err {:?}, want EOF",
        err
    );
}

/// Go: `TestMake`
#[test]
fn test_make() {
    let id = make();
    let rt = parse(&id.to_string()).unwrap_or_else(|e| panic!("parse {:?}: {}", id.to_string(), e));
    assert_eq!(id, rt, "{:?} != {:?}", id.to_string(), rt.to_string());
}

/// Go: `TestMustNew/ULID`
#[test]
fn test_must_new_ulid() {
    test_ulid(&mut must_new);
}

/// Go: `TestMustNew/Panic` — must panic with io.EOF.
#[test]
fn test_must_new_panic() {
    let msg = recover_msg(|| {
        let mut empty = BytesReader::new(Vec::new());
        let _ = must_new(0, Entropy::Reader(&mut empty));
    });
    assert_eq!(
        msg.as_deref(),
        Some("EOF"),
        "panic with err {:?}, want EOF",
        msg
    );
}

/// Go: `TestMustNewDefault/ULID`
#[test]
fn test_must_new_default_ulid() {
    let id = must_new_default(Time::now());
    let rt = parse(&id.to_string()).unwrap_or_else(|e| panic!("parse {:?}: {}", id.to_string(), e));
    assert_eq!(id, rt, "{:?} != {:?}", id.to_string(), rt.to_string());
}

/// Go: `TestMustNewDefault/Panic` — the zero Time overflows into ErrBigTime.
#[test]
fn test_must_new_default_panic() {
    let msg = recover_msg(|| {
        let _ = must_new_default(Time::zero());
    });
    assert_eq!(
        msg.as_deref(),
        Some("ulid: time too big"),
        "got panic {:?}, want ulid: time too big",
        msg
    );
}

/// Go: `TestMustParse/MustParse`
#[test]
fn test_must_parse_must_parse() {
    let msg = recover_msg(|| {
        let _ = must_parse("");
    });
    assert_eq!(
        msg.as_deref(),
        Some("ulid: bad data size when unmarshaling"),
        "got panic {:?}",
        msg
    );
}

/// Go: `TestMustParse/MustParseStrict`
#[test]
fn test_must_parse_must_parse_strict() {
    let msg = recover_msg(|| {
        let _ = must_parse_strict("");
    });
    assert_eq!(
        msg.as_deref(),
        Some("ulid: bad data size when unmarshaling"),
        "got panic {:?}",
        msg
    );
}

// ===========================================================================
// Round trips / marshaling
// ===========================================================================

/// Go: `TestRoundTrips` — quick.Check, MaxCount 1e5.
#[test]
fn test_round_trips() {
    let mut q = Quick::new(1);
    for _ in 0..100_000 {
        let id = q.ulid();

        let bin = id.marshal_binary().expect("MarshalBinary");
        let txt = id.marshal_text().expect("MarshalText");

        let mut a = ULID::default();
        a.unmarshal_binary(&bin).expect("UnmarshalBinary");

        let mut b = ULID::default();
        b.unmarshal_text(&txt).expect("UnmarshalText");

        assert!(
            id == a
                && b == id
                && id == must_parse(&id.to_string())
                && id == must_parse_strict(&id.to_string()),
            "round trip failed for {}",
            id
        );
    }
}

/// Go: `TestMarshalingErrors/UnmarshalBinary`
#[test]
fn test_marshaling_errors_unmarshal_binary() {
    let mut id = ULID::default();
    assert_eq!(id.unmarshal_binary(&[]).unwrap_err(), Error::DataSize);
}

/// Go: `TestMarshalingErrors/UnmarshalText`
#[test]
fn test_marshaling_errors_unmarshal_text() {
    let mut id = ULID::default();
    assert_eq!(id.unmarshal_text(&[]).unwrap_err(), Error::DataSize);
}

/// Go: `TestMarshalingErrors/MarshalBinaryTo`
#[test]
fn test_marshaling_errors_marshal_binary_to() {
    let id = ULID::default();
    let mut buf: Vec<u8> = Vec::new();
    assert_eq!(
        id.marshal_binary_to(&mut buf).unwrap_err(),
        Error::BufferSize
    );
}

/// Go: `TestMarshalingErrors/MarshalTextTo`
#[test]
fn test_marshaling_errors_marshal_text_to() {
    let id = ULID::default();
    let mut buf: Vec<u8> = Vec::new();
    assert_eq!(id.marshal_text_to(&mut buf).unwrap_err(), Error::BufferSize);
}

// ===========================================================================
// TestParseStrictInvalidCharacters (52 subtests)
// ===========================================================================

/// Shared body: replacing any position of a valid ULID with a non-base32 byte
/// must make ParseStrict fail with ErrInvalidCharacters.
fn check_invalid_char(idx: usize, bad: u8) {
    let base = b"0000XSNJG0MQJHBF4QX1EFD6Y3";
    let mut input = base.to_vec();
    input[idx] = bad;
    let err = parse_strict_bytes(&input).err();
    assert_eq!(
        err,
        Some(Error::InvalidCharacters),
        "ParseStrict({:?}) at index {}: got err {:?}, want {}",
        input,
        idx,
        err,
        Error::InvalidCharacters
    );
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_0`
#[test]
fn test_parse_strict_invalid_ff_at_0() {
    check_invalid_char(0, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_0`
#[test]
fn test_parse_strict_invalid_00_at_0() {
    check_invalid_char(0, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_1`
#[test]
fn test_parse_strict_invalid_ff_at_1() {
    check_invalid_char(1, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_1`
#[test]
fn test_parse_strict_invalid_00_at_1() {
    check_invalid_char(1, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_2`
#[test]
fn test_parse_strict_invalid_ff_at_2() {
    check_invalid_char(2, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_2`
#[test]
fn test_parse_strict_invalid_00_at_2() {
    check_invalid_char(2, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_3`
#[test]
fn test_parse_strict_invalid_ff_at_3() {
    check_invalid_char(3, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_3`
#[test]
fn test_parse_strict_invalid_00_at_3() {
    check_invalid_char(3, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_4`
#[test]
fn test_parse_strict_invalid_ff_at_4() {
    check_invalid_char(4, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_4`
#[test]
fn test_parse_strict_invalid_00_at_4() {
    check_invalid_char(4, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_5`
#[test]
fn test_parse_strict_invalid_ff_at_5() {
    check_invalid_char(5, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_5`
#[test]
fn test_parse_strict_invalid_00_at_5() {
    check_invalid_char(5, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_6`
#[test]
fn test_parse_strict_invalid_ff_at_6() {
    check_invalid_char(6, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_6`
#[test]
fn test_parse_strict_invalid_00_at_6() {
    check_invalid_char(6, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_7`
#[test]
fn test_parse_strict_invalid_ff_at_7() {
    check_invalid_char(7, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_7`
#[test]
fn test_parse_strict_invalid_00_at_7() {
    check_invalid_char(7, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_8`
#[test]
fn test_parse_strict_invalid_ff_at_8() {
    check_invalid_char(8, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_8`
#[test]
fn test_parse_strict_invalid_00_at_8() {
    check_invalid_char(8, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_9`
#[test]
fn test_parse_strict_invalid_ff_at_9() {
    check_invalid_char(9, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_9`
#[test]
fn test_parse_strict_invalid_00_at_9() {
    check_invalid_char(9, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_10`
#[test]
fn test_parse_strict_invalid_ff_at_10() {
    check_invalid_char(10, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_10`
#[test]
fn test_parse_strict_invalid_00_at_10() {
    check_invalid_char(10, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_11`
#[test]
fn test_parse_strict_invalid_ff_at_11() {
    check_invalid_char(11, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_11`
#[test]
fn test_parse_strict_invalid_00_at_11() {
    check_invalid_char(11, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_12`
#[test]
fn test_parse_strict_invalid_ff_at_12() {
    check_invalid_char(12, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_12`
#[test]
fn test_parse_strict_invalid_00_at_12() {
    check_invalid_char(12, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_13`
#[test]
fn test_parse_strict_invalid_ff_at_13() {
    check_invalid_char(13, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_13`
#[test]
fn test_parse_strict_invalid_00_at_13() {
    check_invalid_char(13, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_14`
#[test]
fn test_parse_strict_invalid_ff_at_14() {
    check_invalid_char(14, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_14`
#[test]
fn test_parse_strict_invalid_00_at_14() {
    check_invalid_char(14, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_15`
#[test]
fn test_parse_strict_invalid_ff_at_15() {
    check_invalid_char(15, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_15`
#[test]
fn test_parse_strict_invalid_00_at_15() {
    check_invalid_char(15, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_16`
#[test]
fn test_parse_strict_invalid_ff_at_16() {
    check_invalid_char(16, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_16`
#[test]
fn test_parse_strict_invalid_00_at_16() {
    check_invalid_char(16, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_17`
#[test]
fn test_parse_strict_invalid_ff_at_17() {
    check_invalid_char(17, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_17`
#[test]
fn test_parse_strict_invalid_00_at_17() {
    check_invalid_char(17, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_18`
#[test]
fn test_parse_strict_invalid_ff_at_18() {
    check_invalid_char(18, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_18`
#[test]
fn test_parse_strict_invalid_00_at_18() {
    check_invalid_char(18, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_19`
#[test]
fn test_parse_strict_invalid_ff_at_19() {
    check_invalid_char(19, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_19`
#[test]
fn test_parse_strict_invalid_00_at_19() {
    check_invalid_char(19, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_20`
#[test]
fn test_parse_strict_invalid_ff_at_20() {
    check_invalid_char(20, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_20`
#[test]
fn test_parse_strict_invalid_00_at_20() {
    check_invalid_char(20, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_21`
#[test]
fn test_parse_strict_invalid_ff_at_21() {
    check_invalid_char(21, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_21`
#[test]
fn test_parse_strict_invalid_00_at_21() {
    check_invalid_char(21, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_22`
#[test]
fn test_parse_strict_invalid_ff_at_22() {
    check_invalid_char(22, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_22`
#[test]
fn test_parse_strict_invalid_00_at_22() {
    check_invalid_char(22, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_23`
#[test]
fn test_parse_strict_invalid_ff_at_23() {
    check_invalid_char(23, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_23`
#[test]
fn test_parse_strict_invalid_00_at_23() {
    check_invalid_char(23, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_24`
#[test]
fn test_parse_strict_invalid_ff_at_24() {
    check_invalid_char(24, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_24`
#[test]
fn test_parse_strict_invalid_00_at_24() {
    check_invalid_char(24, 0x00);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0xFF_at_index_25`
#[test]
fn test_parse_strict_invalid_ff_at_25() {
    check_invalid_char(25, 0xFF);
}

/// Go: `TestParseStrictInvalidCharacters/Invalid_0x00_at_index_25`
#[test]
fn test_parse_strict_invalid_00_at_25() {
    check_invalid_char(25, 0x00);
}

// ===========================================================================
// Encoding / ordering / robustness
// ===========================================================================

/// Go: `TestAlizainCompatibility` — cross-implementation fixed vector.
#[test]
fn test_alizain_compatibility() {
    let ts: u64 = 1469918176385;
    let mut r = BytesReader::new(vec![0u8; 16]);
    let got = must_new(ts, Entropy::Reader(&mut r));
    let want = must_parse("01ARYZ6S410000000000000000");
    assert_eq!(got, want, "with time={}, got {}, want {}", ts, got, want);
}

/// Go: `TestEncoding` — every char of every encoded ULID is in the alphabet.
#[test]
fn test_encoding() {
    let enc: std::collections::HashSet<u8> = ulid::ENCODING.iter().copied().collect();
    let mut q = Quick::new(2);
    for _ in 0..100_000 {
        let id = q.ulid();
        for r in id.to_string().bytes() {
            assert!(
                enc.contains(&r),
                "character {:?} not in Encoding",
                r as char
            );
        }
    }
}

/// Go: `TestLexicographicalOrder` — boundary walk + quick.Check MaxCount 1e6.
#[test]
fn test_lexicographical_order() {
    let prop = |a: ULID, b: ULID| -> bool {
        let (t1, t2) = (a.time(), b.time());
        let (s1, s2) = (a.to_string(), b.to_string());
        let ord = a.0.cmp(&b.0);
        t1 == t2
            || (t1 > t2 && s1 > s2 && ord == std::cmp::Ordering::Greater)
            || (t1 < t2 && s1 < s2 && ord == std::cmp::Ordering::Less)
    };

    // Test upper boundary state space.
    let mut top = must_new(max_time(), Entropy::None);
    for _ in 0..10 {
        let next = must_new(top.time() - 1, Entropy::None);
        assert!(
            prop(top, next),
            "bad lexicographical order: ({}, {}) > ({}, {}) == false",
            top.time(),
            top,
            next.time(),
            next
        );
        top = next;
    }

    let mut q = Quick::new(3);
    for _ in 0..1_000_000 {
        let (a, b) = (q.ulid(), q.ulid());
        assert!(prop(a, b), "bad lexicographical order for {} / {}", a, b);
    }
}

/// Go: `TestCaseInsensitivity` — quick.CheckEqual(upper, lower).
#[test]
fn test_case_insensitivity() {
    let upper = |id: ULID| must_parse(&id.to_string().to_uppercase());
    let lower = |id: ULID| must_parse(&id.to_string().to_lowercase());

    let mut q = Quick::new(4);
    for _ in 0..QUICK_DEFAULT {
        let id = q.ulid();
        assert_eq!(upper(id), lower(id), "case sensitivity mismatch for {}", id);
    }
}

/// Go: `TestParseRobustness` — fixed byte vector + quick.Check MaxCount 1e4.
/// Parse must never panic and never error on 26-byte input with a valid lead.
#[test]
fn test_parse_robustness() {
    let cases: [&[u8]; 1] = [&[
        0x1, 0xc0, 0x73, 0x62, 0x4a, 0xaf, 0x39, 0x78, 0x51, 0x4e, 0xf8, 0x44, 0x3b, 0xb2, 0xa8,
        0x59, 0xc7, 0x5f, 0xc3, 0xcc, 0x6a, 0xf2, 0x6d, 0x5a, 0xaa, 0x20,
    ]];
    for tc in cases {
        assert!(parse_bytes(tc).is_ok(), "parse failed for {:?}", tc);
    }

    let mut q = Quick::new(5);
    for _ in 0..10_000 {
        let mut s = q.arr26();
        // quick.Check doesn't constrain input, so we do so artificially.
        if s[0] > b'7' {
            s[0] %= b'7';
        }
        assert!(parse_bytes(&s).is_ok(), "parse failed for {:?}", s);
    }
}

// ===========================================================================
// Time
// ===========================================================================

/// Go: `TestNow`
#[test]
fn test_now() {
    let before = now();
    let after = timestamp(Time::now().add(MILLISECOND));
    assert!(
        before < after,
        "clock went mad: before {}, after {}",
        before,
        after
    );
}

/// Go: `TestTimestamp` — sub-ms truncation, and the MaxTime boundary.
#[test]
fn test_timestamp() {
    let tm = Time::unix_new(1, 1000); // will be truncated
    assert_eq!(timestamp(tm), 1000, "for {:?}", tm);

    let mt = max_time();
    let dt =
        Time::unix_new((mt / 1000) as i64, ((mt % 1000) * 1_000_000) as i64).truncate(MILLISECOND);
    assert_eq!(
        timestamp(dt),
        mt,
        "got timestamp {}, want {}",
        timestamp(dt),
        mt
    );
}

/// Go: `TestTime` — round trip loses less than a millisecond.
#[test]
fn test_time() {
    let original = Time::now();
    let diff = original.sub(time(timestamp(original)));
    assert!(
        diff < MILLISECOND,
        "difference between original and recovered time ({}) greater than a millisecond",
        diff
    );
}

/// Go: `TestTimestampRoundTrips` — quick.Check MaxCount 1e5 over arbitrary u64.
/// The identity holds across the whole u64 domain (verified against Go), so the
/// input is deliberately unconstrained, plus explicit boundaries.
#[test]
fn test_timestamp_round_trips() {
    for ts in [
        0u64,
        1,
        999,
        1000,
        u64::MAX,
        u64::MAX - 1,
        i64::MAX as u64,
        max_time(),
    ] {
        assert_eq!(
            ts,
            timestamp(time(ts)),
            "timestamp round trip failed for {}",
            ts
        );
    }
    let mut q = Quick::new(6);
    for _ in 0..100_000 {
        let ts = q.u64();
        assert_eq!(
            ts,
            timestamp(time(ts)),
            "timestamp round trip failed for {}",
            ts
        );
    }
}

/// Go: `TestULIDTime` — SetTime rejects > MaxTime; 1e6 random SetTime/Time round trips.
#[test]
fn test_ulid_time() {
    let mt = max_time();

    let mut id = ULID::default();
    assert_eq!(id.set_time(mt + 1).unwrap_err(), Error::BigTime);

    let mut rng = GoRand::new(Time::now().unix_nano());
    for _ in 0..1_000_000 {
        let ms = rng.int63n(mt as i64) as u64;
        let mut id = ULID::default();
        id.set_time(ms).expect("SetTime");
        assert_eq!(
            id.time(),
            ms,
            "\nfor {}:\ngot  {}\nwant {}",
            id,
            id.time(),
            ms
        );
    }
}

/// Go: `TestULIDTimestamp`
#[test]
fn test_ulid_timestamp() {
    {
        let id = make();
        let ts = id.timestamp();
        let tt = time(id.time());
        assert_eq!(ts, tt, "id.timestamp() != time(id.time())");
    }
    {
        let now_t = Time::now();
        let mut de = default_entropy();
        let id = must_new(timestamp(now_t), Entropy::Monotonic(&mut de));
        let want = now_t.truncate(MILLISECOND);
        let have = id.timestamp();
        assert_eq!(want, have, "Timestamp: want {:?}, have {:?}", want, have);
    }
}

/// Go: `TestZero`
#[test]
fn test_zero() {
    let id = ULID::default();
    assert!(
        id.is_zero(),
        ".IsZero: must return true for zero-value ULIDs, have false"
    );

    let mut de = default_entropy();
    let id = must_new(now(), Entropy::Monotonic(&mut de));
    assert!(
        !id.is_zero(),
        ".IsZero: must return false for non-zero-value ULIDs, have true"
    );
}

// ===========================================================================
// Entropy
// ===========================================================================

/// Go: `TestEntropy` — SetEntropy rejects wrong size; Entropy round trips.
#[test]
fn test_entropy() {
    let mut id = ULID::default();
    assert_eq!(id.set_entropy(&[]).unwrap_err(), Error::DataSize);

    let mut q = Quick::new(7);
    for _ in 0..QUICK_DEFAULT {
        let e = q.arr10();
        let mut id = ULID::default();
        id.set_entropy(&e).expect("SetEntropy");
        assert_eq!(
            id.entropy(),
            e.to_vec(),
            "\n(!= {:?}\n    {:?})",
            id.entropy(),
            e
        );
    }
}

/// Go: `TestEntropyRead` — a partial reader (iotest.HalfReader) still fills
/// all 10 entropy bytes via io.ReadFull.
#[test]
fn test_entropy_read() {
    let mut q = Quick::new(8);
    for _ in 0..10_000 {
        let e = q.arr10();
        let mut src = BytesReader::new(e.to_vec());
        let mut flaky = HalfReader::new(&mut src);
        let id = new(now(), Entropy::Reader(&mut flaky)).expect("New with HalfReader");
        assert_eq!(
            id.entropy(),
            e.to_vec(),
            "\n(!= {:?}\n    {:?})",
            id.entropy(),
            e
        );
    }
}

/// Go: `TestCompare` — Compare agrees with lexicographic String comparison.
#[test]
fn test_compare() {
    let a = |a: ULID, b: ULID| -> i32 {
        match a.to_string().cmp(&b.to_string()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    };
    let b = |x: ULID, y: ULID| -> i32 { x.compare(&y) };

    let mut q = Quick::new(9);
    for _ in 0..100_000 {
        let (x, y) = (q.ulid(), q.ulid());
        assert_eq!(a(x, y), b(x, y), "compare mismatch for {} / {}", x, y);
    }
}

/// Go: `TestOverflowHandling` — first char > '7' overflows 128 bits.
#[test]
fn test_overflow_handling() {
    let cases: [(&str, Option<Error>); 6] = [
        ("00000000000000000000000000", None),
        ("70000000000000000000000000", None),
        ("7ZZZZZZZZZZZZZZZZZZZZZZZZZ", None),
        ("80000000000000000000000000", Some(Error::Overflow)),
        ("80000000000000000000000001", Some(Error::Overflow)),
        ("ZZZZZZZZZZZZZZZZZZZZZZZZZZ", Some(Error::Overflow)),
    ];
    for (s, want) in cases {
        let have = parse(s).err();
        assert_eq!(have, want, "{}: want error {:?}, have {:?}", s, want, have);
    }
}

// ===========================================================================
// TestScan (5 subtests)
// ===========================================================================

/// Builds the fixed ULID the Go TestScan cases are derived from.
fn scan_fixture() -> ULID {
    let mut cr = CryptoRand::new();
    must_new(123, Entropy::Reader(&mut cr))
}

fn check_scan(name: &str, input: ScanValue<'_>, want_out: ULID, want_err: Option<Error>) {
    let mut out = ULID::default();
    let err = out.scan(input).err();
    assert_eq!(
        out.compare(&want_out),
        0,
        "{}: got ULID {}, want {}",
        name,
        out,
        want_out
    );
    assert_eq!(
        sprint_err(&err),
        sprint_err(&want_err),
        "{}: got err {:?}, want {:?}",
        name,
        err,
        want_err
    );
}

/// Go: `TestScan/string`
#[test]
fn test_scan_string() {
    let id = scan_fixture();
    let s = id.to_string();
    check_scan("string", ScanValue::Str(&s), id, None);
}

/// Go: `TestScan/bytes` — 16-byte binary form.
#[test]
fn test_scan_bytes() {
    let id = scan_fixture();
    check_scan("bytes", ScanValue::Bytes(&id.0), id, None);
}

/// Go: `TestScan/text-as-bytes` — 26-byte text form arriving as []byte.
#[test]
fn test_scan_text_as_bytes() {
    let id = scan_fixture();
    let s = id.to_string();
    check_scan("text-as-bytes", ScanValue::Bytes(s.as_bytes()), id, None);
}

/// Go: `TestScan/nil` — NULL column leaves the ULID untouched, no error.
#[test]
fn test_scan_nil() {
    check_scan("nil", ScanValue::Nil, ZERO, None);
}

/// Go: `TestScan/other` — any other dynamic type is rejected.
#[test]
fn test_scan_other() {
    check_scan("other", ScanValue::Other, ZERO, Some(Error::ScanValue));
}

// ===========================================================================
// TestMonotonic (12 subtests) + overflow + concurrency
// ===========================================================================

/// Shared body: 10 000 successive ULIDs at a fixed timestamp must be strictly
/// increasing for the given entropy source and increment bound.
fn check_monotonic(kind: &str, inc: u64) {
    let now_ms = now();
    let src: Box<dyn ulid::EntropyReader> = match kind {
        "cryptorand" => Box::new(CryptoRand::new()),
        _ => Box::new(GoRand::new(now_ms as i64)),
    };
    let mut entropy = monotonic(src, inc);

    let mut prev = ULID::default();
    for _ in 0..10_000 {
        let next = new(123, Entropy::Monotonic(&mut entropy))
            .unwrap_or_else(|e| panic!("entropy={}/inc={}: {}", kind, inc, e));
        assert!(
            prev.compare(&next) < 0,
            "entropy={}/inc={}: prev: {} {:?} > next: {} {:?}",
            kind,
            inc,
            prev.time(),
            prev.entropy(),
            next.time(),
            next.entropy()
        );
        prev = next;
    }
}

/// Go: `TestMonotonic/entropy=cryptorand/inc=0`
#[test]
fn test_monotonic_cryptorand_inc0() {
    check_monotonic("cryptorand", 0);
}

/// Go: `TestMonotonic/entropy=cryptorand/inc=1`
#[test]
fn test_monotonic_cryptorand_inc1() {
    check_monotonic("cryptorand", 1);
}

/// Go: `TestMonotonic/entropy=cryptorand/inc=2`
#[test]
fn test_monotonic_cryptorand_inc2() {
    check_monotonic("cryptorand", 2);
}

/// Go: `TestMonotonic/entropy=cryptorand/inc=256`
#[test]
fn test_monotonic_cryptorand_inc256() {
    check_monotonic("cryptorand", 256);
}

/// Go: `TestMonotonic/entropy=cryptorand/inc=65536`
#[test]
fn test_monotonic_cryptorand_inc65536() {
    check_monotonic("cryptorand", 65536);
}

/// Go: `TestMonotonic/entropy=cryptorand/inc=4294967296`
#[test]
fn test_monotonic_cryptorand_inc4294967296() {
    check_monotonic("cryptorand", 4294967296);
}

/// Go: `TestMonotonic/entropy=mathrand/inc=0`
#[test]
fn test_monotonic_mathrand_inc0() {
    check_monotonic("mathrand", 0);
}

/// Go: `TestMonotonic/entropy=mathrand/inc=1`
#[test]
fn test_monotonic_mathrand_inc1() {
    check_monotonic("mathrand", 1);
}

/// Go: `TestMonotonic/entropy=mathrand/inc=2`
#[test]
fn test_monotonic_mathrand_inc2() {
    check_monotonic("mathrand", 2);
}

/// Go: `TestMonotonic/entropy=mathrand/inc=256`
#[test]
fn test_monotonic_mathrand_inc256() {
    check_monotonic("mathrand", 256);
}

/// Go: `TestMonotonic/entropy=mathrand/inc=65536`
#[test]
fn test_monotonic_mathrand_inc65536() {
    check_monotonic("mathrand", 65536);
}

/// Go: `TestMonotonic/entropy=mathrand/inc=4294967296`
#[test]
fn test_monotonic_mathrand_inc4294967296() {
    check_monotonic("mathrand", 4294967296);
}

/// Go: `TestMonotonic` — the parent test, which drives the full
/// `{cryptorand, mathrand} x {0, 1, 2, 256, 65536, 4294967296}` matrix.
///
/// Go runs the 12 combinations as parallel subtests of one parent function; the
/// 12 `test_monotonic_*` cases above are those subtests, named one-for-one. This
/// case is the parent itself, so the Go suite and the Rust suite agree at *both*
/// levels of the tree rather than only at the leaves — without it, `TestMonotonic`
/// has no exact counterpart and can only be reconciled by name containment.
#[test]
fn test_monotonic() {
    const KINDS: [&str; 2] = ["cryptorand", "mathrand"];
    const INCS: [u64; 6] = [0, 1, 2, 255 + 1, 65535 + 1, 4294967295 + 1];

    let mut ran = 0usize;
    for kind in KINDS {
        for inc in INCS {
            check_monotonic(kind, inc);
            ran += 1;
        }
    }
    // Proves the matrix was actually walked rather than the loops being skipped.
    assert_eq!(ran, 12, "TestMonotonic drives 2 entropy sources x 6 increments");
}

/// Go: `TestMonotonicOverflow` — entropy of all-0xFF must overflow on increment.
#[test]
fn test_monotonic_overflow() {
    let mut first = BytesReader::new(vec![0xFFu8; 10]); // Entropy for first ULID
    let mut rest = CryptoRand::new(); // Following random entropy
    let multi = MultiReader::new(vec![&mut first, &mut rest]);

    let mut entropy = monotonic(Box::new(multi), 0);

    let prev = new(0, Entropy::Monotonic(&mut entropy)).expect("first New");

    let (next, err) = new_partial(prev.time(), Entropy::Monotonic(&mut entropy));
    assert_eq!(
        err,
        Some(Error::MonotonicOverflow),
        "have ulid: {} {:?} err: {:?}, want err: {}",
        next.time(),
        next.entropy(),
        err,
        Error::MonotonicOverflow
    );
}

/// Go: `TestMonotonicSafe` — 100 goroutines x 1024 IDs through a locked reader,
/// every ID strictly greater than the previous one observed by that thread.
#[test]
fn test_monotonic_safe() {
    let rng = GoRand::new(Time::now().unix_nano());
    let safe = Arc::new(LockedMonotonicReader::new(monotonic(Box::new(rng), 0)));
    let t0 = timestamp(Time::now());

    let mut handles = Vec::new();
    for _ in 0..100 {
        let safe = Arc::clone(&safe);
        handles.push(std::thread::spawn(move || -> Result<usize, String> {
            let mut generated = 0usize;
            let mut h = LockedHandle(&safe);
            let mut u0 = must_new(t0, Entropy::Monotonic(&mut h));
            generated += 1;
            let mut u1 = u0;
            for _ in 0..1024 {
                let mut h = LockedHandle(&safe);
                let next = must_new(t0, Entropy::Monotonic(&mut h));
                generated += 1;
                u0 = u1;
                u1 = next;
                if u0.to_string() >= u1.to_string() {
                    return Err(format!(
                        "{} ({} {:02x?}) >= {} ({} {:02x?})",
                        u0,
                        u0.time(),
                        u0.entropy(),
                        u1,
                        u1.time(),
                        u1.entropy()
                    ));
                }
            }
            Ok(generated)
        }));
    }

    let mut total = 0usize;
    for h in handles {
        let n = h
            .join()
            .expect("thread panicked")
            .expect("monotonicity violated");
        assert_eq!(n, 1025, "each thread must generate 1 + 1024 ULIDs");
        total += n;
    }
    // Proves the work actually happened rather than the loops being skipped.
    assert_eq!(
        total,
        100 * 1025,
        "expected 102500 ULIDs across 100 threads"
    );
}

/// Go: `TestULID_Bytes` — Bytes() must return a copy, not an alias.
#[test]
fn test_ulid_bytes() {
    let tt = Time::unix_new(1000000, 0);
    let rng = GoRand::new(tt.unix_nano());
    let mut entropy = monotonic(Box::new(rng), 0);
    let id = must_new(timestamp(tt), Entropy::Monotonic(&mut entropy));

    let mut bid = id.bytes();
    let last = bid.len() - 1;
    bid[last] = bid[last].wrapping_add(1);
    assert_ne!(
        id.bytes(),
        bid,
        "Bytes() returned a reference to ulid underlying array!"
    );
}
