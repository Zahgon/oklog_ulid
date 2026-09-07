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

//! A 1:1 Rust port of `github.com/oklog/ulid/v2`.
//!
//! Every exported Go identifier has an equivalent here, and the bit-level
//! encoding, error values, overflow rules and monotonic-entropy behaviour are
//! reproduced exactly.

pub mod goio;
pub mod gorand;
pub mod gotime;
mod rng_cooked;

// ---------------------------------------------------------------------------
// Crate-root re-exports
// ---------------------------------------------------------------------------
//
// Everything a consumer (or a test) needs is reachable from the crate root, so
// no caller has to name an internal module path. The modules above remain
// public for callers who prefer the qualified form.

pub use goio::{
    read_at_least, read_full, BufCore, BytesReader, CryptoRand, HalfReader, IoError, MultiReader,
    Reader, ZeroReader,
};
pub use gorand::{GoRand, Int63nSource, RngSource};
pub use gotime::{
    civil_to_unix, days_from_civil, format_civil, offset_between, Civil, Time, LAYOUT_DEFAULT_MS,
    LAYOUT_RFC3339_MS, MILLISECOND, SECOND,
};

// `read_full_buffered` is internal-only; everything else arrives via the
// crate-root re-exports above.
use goio::read_full_buffered;
use std::fmt;
use std::sync::{Mutex, OnceLock};

/// A ULID is a 16 byte Universally Unique Lexicographically Sortable Identifier
///
/// ```text
/// The components are encoded as 16 octets.
/// Each component is encoded with the MSB first (network byte order).
///
/// 0                   1                   2                   3
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                      32_bit_uint_time_high                    |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |     16_bit_uint_time_low      |       16_bit_uint_random      |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                       32_bit_uint_random                      |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                       32_bit_uint_random                      |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ULID(pub [u8; 16]);

/// Port of the package-level `error` values in `ulid.go`.
///
/// The `Display` text of each variant matches the Go message byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `ErrDataSize`: parsing/unmarshaling a ULID with the wrong data size.
    DataSize,
    /// `ErrInvalidCharacters`: invalid Base32 encoding.
    InvalidCharacters,
    /// `ErrBufferSize`: marshalling into a buffer of insufficient size.
    BufferSize,
    /// `ErrBigTime`: constructing a ULID with a time larger than `MaxTime`.
    BigTime,
    /// `ErrOverflow`: first character larger than 7, exceeding 128 bits.
    Overflow,
    /// `ErrMonotonicOverflow`: incrementing entropy bytes would overflow.
    MonotonicOverflow,
    /// `ErrScanValue`: value passed to `Scan` cannot be unmarshaled.
    ScanValue,
    /// An error surfaced from the entropy `io.Reader`.
    Io(IoError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DataSize => write!(f, "ulid: bad data size when unmarshaling"),
            Error::InvalidCharacters => write!(f, "ulid: bad data characters when unmarshaling"),
            Error::BufferSize => write!(f, "ulid: bad buffer size when marshaling"),
            Error::BigTime => write!(f, "ulid: time too big"),
            Error::Overflow => write!(f, "ulid: overflow when unmarshaling"),
            Error::MonotonicOverflow => write!(f, "ulid: monotonic entropy overflow"),
            Error::ScanValue => write!(f, "ulid: source value must be a string or byte slice"),
            Error::Io(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for Error {}

/// A zero-value ULID (Go's `ulid.Zero`).
pub const ZERO: ULID = ULID([0u8; 16]);

/// `EncodedSize` is the length of a text encoded ULID.
pub const ENCODED_SIZE: usize = 26;

/// `Encoding` is the base 32 encoding alphabet used in ULID strings.
pub const ENCODING: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Byte to index table for O(1) lookups when unmarshaling.
/// `0xFF` is the sentinel value for invalid indexes.
#[rustfmt::skip]
const DEC: [u8; 256] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x01,
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
    0x0F, 0x10, 0x11, 0xFF, 0x12, 0x13, 0xFF, 0x14, 0x15, 0xFF,
    0x16, 0x17, 0x18, 0x19, 0x1A, 0xFF, 0x1B, 0x1C, 0x1D, 0x1E,
    0x1F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0A, 0x0B, 0x0C,
    0x0D, 0x0E, 0x0F, 0x10, 0x11, 0xFF, 0x12, 0x13, 0xFF, 0x14,
    0x15, 0xFF, 0x16, 0x17, 0x18, 0x19, 0x1A, 0xFF, 0x1B, 0x1C,
    0x1D, 0x1E, 0x1F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// `maxTime` is the maximum Unix time in milliseconds representable in a ULID.
const MAX_TIME: u64 = 0x0000_FFFF_FFFF_FFFF; // ULID{0xFF x6}.Time()

/// `MaxTime` returns the maximum Unix time in milliseconds that
/// can be encoded in a ULID.
pub fn max_time() -> u64 {
    MAX_TIME
}

// --------------------------------------------------------------------------
// Entropy sources
// --------------------------------------------------------------------------

/// Port of Go's `MonotonicReader` interface.
///
/// Should yield monotonically increasing entropy into the provided slice for
/// all calls with the same `ms` parameter.
pub trait MonotonicReader: Reader {
    fn monotonic_read(&mut self, ms: u64, p: &mut [u8]) -> Result<(), Error>;
}

/// An entropy source that may additionally satisfy Go's unexported
/// `rng interface{ Int63n(int64) int64 }`, which `Monotonic` type-asserts
/// against to select a faster increment path.
pub trait EntropyReader: Reader {
    fn as_int63n(&mut self) -> Option<&mut dyn Int63nSource> {
        None
    }
}

impl EntropyReader for gorand::GoRand {
    fn as_int63n(&mut self) -> Option<&mut dyn Int63nSource> {
        Some(self)
    }
}
impl EntropyReader for goio::CryptoRand {}
impl EntropyReader for goio::BytesReader {}
impl EntropyReader for goio::ZeroReader {}
impl<'a> EntropyReader for goio::MultiReader<'a> {}
impl<'a> EntropyReader for goio::HalfReader<'a> {}

/// The `entropy io.Reader` argument of `New`/`MustNew`.
///
/// Go dispatches on the dynamic type of the argument (`nil`, `MonotonicReader`,
/// or a plain `io.Reader`); this enum makes that dispatch explicit.
pub enum Entropy<'a> {
    /// Go's `nil` entropy: leaves the entropy bytes zeroed.
    None,
    /// A plain `io.Reader`.
    Reader(&'a mut dyn Reader),
    /// A `MonotonicReader`, which takes the `MonotonicRead` path.
    Monotonic(&'a mut dyn MonotonicReader),
}

// --------------------------------------------------------------------------
// Constructors
// --------------------------------------------------------------------------

/// `New` returns a ULID with the given Unix milliseconds timestamp and an
/// optional entropy source. Use `timestamp` to convert a `Time` to Unix
/// milliseconds.
///
/// `Error::BigTime` is returned when passing a timestamp bigger than `max_time()`.
/// Reading from the entropy source may also return an error.
pub fn new(ms: u64, entropy: Entropy<'_>) -> Result<ULID, Error> {
    let (id, err) = new_partial(ms, entropy);
    match err {
        Some(e) => Err(e),
        None => Ok(id),
    }
}

/// Exactly Go's `New`, including its `(id, err)` tuple: on failure the
/// partially-populated ULID is still returned (the Go tests observe this).
pub fn new_partial(ms: u64, entropy: Entropy<'_>) -> (ULID, Option<Error>) {
    let mut id = ULID([0u8; 16]);
    if let Err(e) = id.set_time(ms) {
        return (id, Some(e));
    }

    let err = match entropy {
        Entropy::None => None,
        Entropy::Monotonic(e) => e.monotonic_read(ms, &mut id.0[6..]).err(),
        Entropy::Reader(e) => {
            let (_, err) = read_full(e, &mut id.0[6..]);
            err.map(Error::Io)
        }
    };

    (id, err)
}

/// `MustNew` is a convenience function equivalent to `new` that panics on
/// failure instead of returning an error.
pub fn must_new(ms: u64, entropy: Entropy<'_>) -> ULID {
    match new(ms, entropy) {
        Ok(id) => id,
        Err(e) => panic!("{}", PanicErr(e)),
    }
}

/// `MustNewDefault` is a convenience function equivalent to `must_new` with
/// `default_entropy()` as the entropy. It may panic if the given time is too
/// large or too small.
pub fn must_new_default(t: Time) -> ULID {
    let mut de = DefaultEntropy;
    must_new(timestamp(t), Entropy::Monotonic(&mut de))
}

/// `Make` returns a ULID with the current time in Unix milliseconds and
/// monotonically increasing entropy for the same millisecond.
/// It is safe for concurrent use.
pub fn make() -> ULID {
    // NOTE: must_new can't panic since default entropy never returns an error.
    let mut de = DefaultEntropy;
    must_new(now(), Entropy::Monotonic(&mut de))
}

/// Wrapper so `panic!` payloads carry the `Error`, letting ported tests
/// recover it the way Go's `recover()` does.
pub struct PanicErr(pub Error);

impl fmt::Display for PanicErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// --------------------------------------------------------------------------
// Parsing
// --------------------------------------------------------------------------

/// `Parse` parses an encoded ULID, returning an error in case of failure.
///
/// `Error::DataSize` is returned if the length differs from an encoded ULID's
/// length. Invalid encodings produce undefined ULIDs. For a version that
/// returns an error instead, see `parse_strict`.
pub fn parse(s: &str) -> Result<ULID, Error> {
    parse_bytes(s.as_bytes())
}

/// Byte-oriented `Parse`, needed because the Go tests feed in byte sequences
/// that are not valid UTF-8.
pub fn parse_bytes(v: &[u8]) -> Result<ULID, Error> {
    let mut id = ULID([0u8; 16]);
    parse_into(v, false, &mut id)?;
    Ok(id)
}

/// `ParseStrict` parses an encoded ULID, returning an error in case of failure.
///
/// Like `parse`, but additionally validates that the parsed ULID consists only
/// of valid base32 characters.
pub fn parse_strict(s: &str) -> Result<ULID, Error> {
    parse_strict_bytes(s.as_bytes())
}

/// Byte-oriented `ParseStrict`.
pub fn parse_strict_bytes(v: &[u8]) -> Result<ULID, Error> {
    let mut id = ULID([0u8; 16]);
    parse_into(v, true, &mut id)?;
    Ok(id)
}

fn parse_into(v: &[u8], strict: bool, id: &mut ULID) -> Result<(), Error> {
    // Check if a base32 encoded ULID is the right length.
    if v.len() != ENCODED_SIZE {
        return Err(Error::DataSize);
    }

    // Check if all the characters in a base32 encoded ULID are part of the
    // expected base32 character set.
    if strict && (0..ENCODED_SIZE).any(|i| DEC[v[i] as usize] == 0xFF) {
        return Err(Error::InvalidCharacters);
    }

    // Check if the first character in a base32 encoded ULID will overflow. This
    // happens because the base32 representation encodes 130 bits, while the
    // ULID is only 128 bits.
    //
    // See https://github.com/oklog/ulid/issues/9 for details.
    if v[0] > b'7' {
        return Err(Error::Overflow);
    }

    // Use an optimized unrolled loop (from https://github.com/RobThree/NUlid)
    // to decode a base32 ULID.
    //
    // Go's `<<` on a byte truncates to 8 bits; Rust panics on overflow in debug
    // builds, so every shift below uses `wrapping_shl` semantics via `u8`.
    let d = |i: usize| DEC[v[i] as usize];

    // 6 bytes timestamp (48 bits)
    id.0[0] = (d(0) << 5) | d(1);
    id.0[1] = (d(2) << 3) | (d(3) >> 2);
    id.0[2] = (d(3) << 6) | (d(4) << 1) | (d(5) >> 4);
    id.0[3] = (d(5) << 4) | (d(6) >> 1);
    id.0[4] = (d(6) << 7) | (d(7) << 2) | (d(8) >> 3);
    id.0[5] = (d(8) << 5) | d(9);

    // 10 bytes of entropy (80 bits)
    id.0[6] = (d(10) << 3) | (d(11) >> 2);
    id.0[7] = (d(11) << 6) | (d(12) << 1) | (d(13) >> 4);
    id.0[8] = (d(13) << 4) | (d(14) >> 1);
    id.0[9] = (d(14) << 7) | (d(15) << 2) | (d(16) >> 3);
    id.0[10] = (d(16) << 5) | d(17);
    id.0[11] = (d(18) << 3) | (d(19) >> 2);
    id.0[12] = (d(19) << 6) | (d(20) << 1) | (d(21) >> 4);
    id.0[13] = (d(21) << 4) | (d(22) >> 1);
    id.0[14] = (d(22) << 7) | (d(23) << 2) | (d(24) >> 3);
    id.0[15] = (d(24) << 5) | d(25);

    Ok(())
}

/// `MustParse` is a convenience function equivalent to `parse` that panics on
/// failure instead of returning an error.
pub fn must_parse(s: &str) -> ULID {
    match parse(s) {
        Ok(id) => id,
        Err(e) => panic!("{}", PanicErr(e)),
    }
}

/// `MustParseStrict` is a convenience function equivalent to `parse_strict`
/// that panics on failure instead of returning an error.
pub fn must_parse_strict(s: &str) -> ULID {
    match parse_strict(s) {
        Ok(id) => id,
        Err(e) => panic!("{}", PanicErr(e)),
    }
}

// --------------------------------------------------------------------------
// Time helpers
// --------------------------------------------------------------------------

/// `Now` is a convenience function that returns the current UTC time in Unix
/// milliseconds.
pub fn now() -> u64 {
    timestamp(Time::now())
}

/// `Timestamp` converts a `Time` to Unix milliseconds.
///
/// Because of the way ULID stores time, times from the year 10889 produce
/// undefined results. Go performs this arithmetic on `uint64` with wraparound,
/// which is what makes the zero `Time` overflow into `ErrBigTime`.
pub fn timestamp(t: Time) -> u64 {
    (t.unix() as u64)
        .wrapping_mul(1000)
        .wrapping_add((t.nanosecond() as i64 / gotime::MILLISECOND) as u64)
}

/// `Time` converts Unix milliseconds in the format returned by `timestamp`
/// back to a `Time`.
pub fn time(ms: u64) -> Time {
    let s = (ms / 1_000) as i64;
    let ns = ((ms % 1_000) * 1_000_000) as i64;
    Time::unix_new(s, ns)
}

// --------------------------------------------------------------------------
// ULID methods
// --------------------------------------------------------------------------

impl ULID {
    /// `Bytes` returns a byte-slice copy of the ULID.
    ///
    /// Go returns `id[:]` from a value receiver, so the caller gets a copy of
    /// the array; `TestULID_Bytes` asserts mutating it does not alias the ULID.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// `MarshalBinary` returns the ULID as a byte slice.
    pub fn marshal_binary(&self) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; 16];
        self.marshal_binary_to(&mut out)?;
        Ok(out)
    }

    /// `MarshalBinaryTo` writes the binary encoding of the ULID to `dst`.
    /// `Error::BufferSize` is returned when `dst.len() != 16`.
    pub fn marshal_binary_to(&self, dst: &mut [u8]) -> Result<(), Error> {
        if dst.len() != self.0.len() {
            return Err(Error::BufferSize);
        }
        dst.copy_from_slice(&self.0);
        Ok(())
    }

    /// `UnmarshalBinary` copies `data` into the ULID.
    /// `Error::DataSize` is returned if the length differs from 16.
    pub fn unmarshal_binary(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.len() != self.0.len() {
            return Err(Error::DataSize);
        }
        self.0.copy_from_slice(data);
        Ok(())
    }

    /// `MarshalText` returns the string encoded ULID.
    pub fn marshal_text(&self) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; ENCODED_SIZE];
        self.marshal_text_to(&mut out)?;
        Ok(out)
    }

    /// `MarshalTextTo` writes the ULID as a string to `dst`.
    /// `Error::BufferSize` is returned when `dst.len() != 26`.
    pub fn marshal_text_to(&self, dst: &mut [u8]) -> Result<(), Error> {
        // Optimized unrolled loop ahead.
        // From https://github.com/RobThree/NUlid
        if dst.len() != ENCODED_SIZE {
            return Err(Error::BufferSize);
        }
        let id = &self.0;
        let e = |i: u8| ENCODING[i as usize];

        // 10 byte timestamp
        dst[0] = e((id[0] & 224) >> 5);
        dst[1] = e(id[0] & 31);
        dst[2] = e((id[1] & 248) >> 3);
        dst[3] = e(((id[1] & 7) << 2) | ((id[2] & 192) >> 6));
        dst[4] = e((id[2] & 62) >> 1);
        dst[5] = e(((id[2] & 1) << 4) | ((id[3] & 240) >> 4));
        dst[6] = e(((id[3] & 15) << 1) | ((id[4] & 128) >> 7));
        dst[7] = e((id[4] & 124) >> 2);
        dst[8] = e(((id[4] & 3) << 3) | ((id[5] & 224) >> 5));
        dst[9] = e(id[5] & 31);

        // 16 bytes of entropy
        dst[10] = e((id[6] & 248) >> 3);
        dst[11] = e(((id[6] & 7) << 2) | ((id[7] & 192) >> 6));
        dst[12] = e((id[7] & 62) >> 1);
        dst[13] = e(((id[7] & 1) << 4) | ((id[8] & 240) >> 4));
        dst[14] = e(((id[8] & 15) << 1) | ((id[9] & 128) >> 7));
        dst[15] = e((id[9] & 124) >> 2);
        dst[16] = e(((id[9] & 3) << 3) | ((id[10] & 224) >> 5));
        dst[17] = e(id[10] & 31);
        dst[18] = e((id[11] & 248) >> 3);
        dst[19] = e(((id[11] & 7) << 2) | ((id[12] & 192) >> 6));
        dst[20] = e((id[12] & 62) >> 1);
        dst[21] = e(((id[12] & 1) << 4) | ((id[13] & 240) >> 4));
        dst[22] = e(((id[13] & 15) << 1) | ((id[14] & 128) >> 7));
        dst[23] = e((id[14] & 124) >> 2);
        dst[24] = e(((id[14] & 3) << 3) | ((id[15] & 224) >> 5));
        dst[25] = e(id[15] & 31);

        Ok(())
    }

    /// `UnmarshalText` parses `v` as a string encoded ULID.
    pub fn unmarshal_text(&mut self, v: &[u8]) -> Result<(), Error> {
        parse_into(v, false, self)
    }

    /// `Time` returns the Unix time in milliseconds encoded in the ULID.
    pub fn time(&self) -> u64 {
        let id = &self.0;
        (id[5] as u64)
            | (id[4] as u64) << 8
            | (id[3] as u64) << 16
            | (id[2] as u64) << 24
            | (id[1] as u64) << 32
            | (id[0] as u64) << 40
    }

    /// `Timestamp` returns the time encoded in the ULID as a `Time`.
    pub fn timestamp(&self) -> Time {
        time(self.time())
    }

    /// `IsZero` returns true if the ULID is a zero-value ULID.
    pub fn is_zero(&self) -> bool {
        self.compare(&ZERO) == 0
    }

    /// `SetTime` sets the time component of the ULID to the given Unix time
    /// in milliseconds.
    pub fn set_time(&mut self, ms: u64) -> Result<(), Error> {
        if ms > MAX_TIME {
            return Err(Error::BigTime);
        }
        self.0[0] = (ms >> 40) as u8;
        self.0[1] = (ms >> 32) as u8;
        self.0[2] = (ms >> 24) as u8;
        self.0[3] = (ms >> 16) as u8;
        self.0[4] = (ms >> 8) as u8;
        self.0[5] = ms as u8;
        Ok(())
    }

    /// `Entropy` returns the entropy from the ULID.
    pub fn entropy(&self) -> Vec<u8> {
        self.0[6..].to_vec()
    }

    /// `SetEntropy` sets the ULID entropy to the passed byte slice.
    /// `Error::DataSize` is returned if `e.len() != 10`.
    pub fn set_entropy(&mut self, e: &[u8]) -> Result<(), Error> {
        if e.len() != 10 {
            return Err(Error::DataSize);
        }
        self.0[6..].copy_from_slice(e);
        Ok(())
    }

    /// `Compare` returns an integer comparing `self` and `other`
    /// lexicographically: 0 if equal, -1 if less, +1 if greater.
    pub fn compare(&self, other: &ULID) -> i32 {
        match self.0.cmp(&other.0) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `Scan` implements `sql.Scanner`. It supports scanning a string or byte
    /// slice.
    pub fn scan(&mut self, src: ScanValue<'_>) -> Result<(), Error> {
        match src {
            ScanValue::Nil => Ok(()),
            ScanValue::Str(s) => self.unmarshal_text(s.as_bytes()),
            ScanValue::Bytes(x) => {
                // Drivers often return text/varchar columns as []byte. Accept
                // both the 16-byte binary form and the 26-character text form.
                match x.len() {
                    16 => self.unmarshal_binary(x),
                    ENCODED_SIZE => self.unmarshal_text(x),
                    _ => Err(Error::DataSize),
                }
            }
            ScanValue::Other => Err(Error::ScanValue),
        }
    }

    /// `Value` implements `driver.Valuer`, returning the ULID as a byte slice
    /// by invoking `marshal_binary`.
    pub fn value(&self) -> Result<Vec<u8>, Error> {
        self.marshal_binary()
    }
}

/// The `interface{}` argument accepted by `Scan`.
pub enum ScanValue<'a> {
    /// Go's `nil`.
    Nil,
    /// A `string` column.
    Str(&'a str),
    /// A `[]byte` column.
    Bytes(&'a [u8]),
    /// Any other dynamic type, which Go rejects with `ErrScanValue`.
    Other,
}

/// `String` returns a lexicographically sortable string encoded ULID
/// (26 characters, non-standard base 32), e.g. `01AN4Z07BY79KA1307SR9X4MV3`.
/// Format: `tttttttttteeeeeeeeeeeeeeee` where `t` is time and `e` is entropy.
impl fmt::Display for ULID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; ENCODED_SIZE];
        let _ = self.marshal_text_to(&mut buf);
        // Every byte is drawn from the base32 alphabet, so this is valid ASCII.
        f.write_str(std::str::from_utf8(&buf).unwrap())
    }
}

// --------------------------------------------------------------------------
// Monotonic entropy
// --------------------------------------------------------------------------

/// `Monotonic` returns a source of entropy that yields strictly increasing
/// entropy bytes, to a limit governed by the `inc` parameter.
///
/// Calls to `monotonic_read` within the same ULID timestamp return entropy
/// incremented by a random number between 1 and `inc` inclusive. If an
/// increment would overflow, `Error::MonotonicOverflow` is returned.
///
/// Passing `inc == 0` results in the default `u32::MAX`.
///
/// The returned type isn't safe for concurrent use.
pub fn monotonic<'a>(entropy: Box<dyn EntropyReader + 'a>, inc: u64) -> MonotonicEntropy<'a> {
    let mut m = MonotonicEntropy {
        src: entropy,
        buf: BufCore::new(),
        ms: 0,
        inc,
        entropy: Uint80 { hi: 0, lo: 0 },
        rand: [0u8; 8],
        use_rng: false,
    };
    if m.inc == 0 {
        m.inc = u32::MAX as u64;
    }
    // Go: `if rng, ok := entropy.(rng); ok { m.rng = rng }`
    m.use_rng = m.src.as_int63n().is_some();
    m
}

/// `MonotonicEntropy` is an opaque type that provides monotonic entropy.
pub struct MonotonicEntropy<'a> {
    src: Box<dyn EntropyReader + 'a>,
    /// Go wraps the source in a `bufio.Reader`; the read-ahead is observable.
    buf: BufCore,
    ms: u64,
    inc: u64,
    entropy: Uint80,
    rand: [u8; 8],
    use_rng: bool,
}

/// Views a `dyn EntropyReader` as a `dyn Reader`.
///
/// `EntropyReader` has `Reader` as a supertrait, but coercing
/// `&mut dyn EntropyReader` to `&mut dyn Reader` is trait upcasting, which is
/// only stable from Rust 1.86. This adapter forwards the one method explicitly
/// so the crate still builds on its declared MSRV.
struct AsReader<'x, 'a>(&'x mut (dyn EntropyReader + 'a));

impl<'x, 'a> Reader for AsReader<'x, 'a> {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        self.0.read(p)
    }
}

impl<'a> Reader for MonotonicEntropy<'a> {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        self.buf.read_from(&mut AsReader(self.src.as_mut()), p)
    }
}

impl<'a> MonotonicReader for MonotonicEntropy<'a> {
    /// Port of `(*MonotonicEntropy).MonotonicRead`.
    fn monotonic_read(&mut self, ms: u64, entropy: &mut [u8]) -> Result<(), Error> {
        if !self.entropy.is_zero() && self.ms == ms {
            let err = self.increment();
            self.entropy.append_to(entropy);
            err
        } else {
            let (_, e) =
                read_full_buffered(&mut self.buf, &mut AsReader(self.src.as_mut()), entropy);
            match e {
                Some(e) => Err(Error::Io(e)),
                None => {
                    self.ms = ms;
                    self.entropy.set_bytes(entropy);
                    Ok(())
                }
            }
        }
    }
}

impl<'a> MonotonicEntropy<'a> {
    /// Increment the previous entropy number with a random number of up to
    /// `self.inc` (inclusive).
    fn increment(&mut self) -> Result<(), Error> {
        match self.random() {
            Err(e) => Err(e),
            Ok(inc) => {
                if self.entropy.add(inc) {
                    Err(Error::MonotonicOverflow)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Returns a uniform random value in `[1, self.inc)`, reading entropy from
    /// the buffered reader. When `inc <= 1`, returns 1.
    /// Adapted from `crypto/rand.Int`.
    fn random(&mut self) -> Result<u64, Error> {
        if self.inc <= 1 {
            return Ok(1);
        }

        // Fast path for using an underlying rand.Rand directly.
        if self.use_rng {
            let inc = self.inc as i64;
            let v = self.src.as_int63n().unwrap().int63n(inc);
            // Range: [1, m.inc)
            return Ok(1 + v as u64);
        }

        // bitLen is the maximum bit length needed to encode a value < m.inc.
        let bit_len = 64 - self.inc.leading_zeros() as usize;
        // byteLen is the maximum byte length needed to encode a value < m.inc.
        let byte_len = (bit_len + 7) / 8;
        // msbitLen is the number of bits in the most significant byte of m.inc-1.
        let mut msbit_len = bit_len % 8;
        if msbit_len == 0 {
            msbit_len = 8;
        }

        let mut inc: u64 = 0;
        while inc == 0 || inc >= self.inc {
            let (_, e) = read_full_buffered(
                &mut self.buf,
                &mut AsReader(self.src.as_mut()),
                &mut self.rand[..byte_len],
            );
            if let Some(e) = e {
                return Err(Error::Io(e));
            }

            // Clear bits in the first byte to increase the probability
            // that the candidate is < m.inc.
            self.rand[0] &= ((1u32 << msbit_len) - 1) as u8;

            // Convert the read bytes into a u64 with byteLen.
            //
            // NOTE: Go widens to the next power-of-two width (3 -> 4 bytes,
            // 5..8 -> 8 bytes) and reads *stale* bytes left in `m.rand` from a
            // previous iteration. That is reproduced exactly. `byte_len` is
            // always 1..=8 here (inc >= 2), so this chain is exhaustive without
            // a dead fallback arm.
            inc = if byte_len <= 1 {
                self.rand[0] as u64
            } else if byte_len == 2 {
                u16::from_le_bytes([self.rand[0], self.rand[1]]) as u64
            } else if byte_len <= 4 {
                u32::from_le_bytes([self.rand[0], self.rand[1], self.rand[2], self.rand[3]]) as u64
            } else {
                u64::from_le_bytes(self.rand)
            };
        }

        // Range: [1, m.inc)
        Ok(1 + inc)
    }
}

/// `LockedMonotonicReader` wraps a `MonotonicReader` with a mutex for safe
/// concurrent use.
pub struct LockedMonotonicReader<'a> {
    inner: Mutex<MonotonicEntropy<'a>>,
}

impl<'a> LockedMonotonicReader<'a> {
    pub fn new(m: MonotonicEntropy<'a>) -> Self {
        LockedMonotonicReader {
            inner: Mutex::new(m),
        }
    }

    /// `MonotonicRead` synchronizes calls to the wrapped `MonotonicReader`.
    pub fn monotonic_read(&self, ms: u64, p: &mut [u8]) -> Result<(), Error> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.monotonic_read(ms, p)
    }

    pub fn read(&self, p: &mut [u8]) -> (usize, Option<IoError>) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.read(p)
    }
}

/// A borrowed handle to a [`LockedMonotonicReader`], so several threads can
/// share one monotonic source (Go passes the `*LockedMonotonicReader` itself,
/// which satisfies `MonotonicReader` directly).
pub struct LockedHandle<'r, 'a>(pub &'r LockedMonotonicReader<'a>);

impl<'r, 'a> Reader for LockedHandle<'r, 'a> {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        self.0.read(p)
    }
}

impl<'r, 'a> MonotonicReader for LockedHandle<'r, 'a> {
    fn monotonic_read(&mut self, ms: u64, p: &mut [u8]) -> Result<(), Error> {
        self.0.monotonic_read(ms, p)
    }
}

// --------------------------------------------------------------------------
// Default entropy
// --------------------------------------------------------------------------

static DEFAULT_ENTROPY: OnceLock<LockedMonotonicReader<'static>> = OnceLock::new();

fn default_entropy_global() -> &'static LockedMonotonicReader<'static> {
    DEFAULT_ENTROPY.get_or_init(|| {
        let seed = Time::now().unix_nano();
        let rng = gorand::GoRand::new(seed);
        LockedMonotonicReader::new(monotonic(Box::new(rng), 0))
    })
}

/// A handle to the thread-safe, per-process monotonically increasing entropy
/// source (Go's `DefaultEntropy()`).
pub struct DefaultEntropy;

impl Reader for DefaultEntropy {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        default_entropy_global().read(p)
    }
}

impl MonotonicReader for DefaultEntropy {
    fn monotonic_read(&mut self, ms: u64, p: &mut [u8]) -> Result<(), Error> {
        default_entropy_global().monotonic_read(ms, p)
    }
}

/// `DefaultEntropy` returns a thread-safe per process monotonically increasing
/// entropy source.
pub fn default_entropy() -> DefaultEntropy {
    DefaultEntropy
}

// --------------------------------------------------------------------------
// uint80
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Uint80 {
    hi: u16,
    lo: u64,
}

impl Uint80 {
    fn set_bytes(&mut self, bs: &[u8]) {
        self.hi = u16::from_be_bytes([bs[0], bs[1]]);
        self.lo = u64::from_be_bytes([bs[2], bs[3], bs[4], bs[5], bs[6], bs[7], bs[8], bs[9]]);
    }

    fn append_to(&self, bs: &mut [u8]) {
        bs[..2].copy_from_slice(&self.hi.to_be_bytes());
        bs[2..10].copy_from_slice(&self.lo.to_be_bytes());
    }

    /// Returns true on overflow, matching Go's carry logic exactly.
    fn add(&mut self, n: u64) -> bool {
        let lo = self.lo;
        let hi = self.hi;
        self.lo = self.lo.wrapping_add(n);
        if self.lo < lo {
            self.hi = self.hi.wrapping_add(1);
        }
        self.hi < hi
    }

    fn is_zero(&self) -> bool {
        self.hi == 0 && self.lo == 0
    }
}
