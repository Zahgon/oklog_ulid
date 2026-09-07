//! Faithful port of the Go `io`/`bufio`/`bytes`/`strings`/`testing/iotest`
//! reader semantics that `ulid` depends on.
//!
//! Go's `Read` returns `(n, err)` where a non-zero `n` may accompany `io.EOF`.
//! That nuance is observable through `io.ReadFull` (which distinguishes `EOF`
//! from `ErrUnexpectedEOF`) and through `bufio.Reader`'s buffering, so the trait
//! below mirrors Go's signature exactly rather than using `std::io::Read`.

use std::fmt;

/// Port of Go's sentinel `io` errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoError {
    /// `io.EOF`
    Eof,
    /// `io.ErrUnexpectedEOF`
    UnexpectedEof,
    /// `io.ErrShortBuffer`
    ShortBuffer,
    /// Any other error, carrying its message.
    Other(String),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::Eof => write!(f, "EOF"),
            IoError::UnexpectedEof => write!(f, "unexpected EOF"),
            IoError::ShortBuffer => write!(f, "short buffer"),
            IoError::Other(s) => write!(f, "{}", s),
        }
    }
}

/// Port of Go's `io.Reader`.
pub trait Reader: Send {
    /// Mirrors Go's `Read(p []byte) (n int, err error)`.
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>);
}

/// Port of `io.ReadAtLeast`.
pub fn read_at_least(r: &mut dyn Reader, buf: &mut [u8], min: usize) -> (usize, Option<IoError>) {
    if buf.len() < min {
        return (0, Some(IoError::ShortBuffer));
    }
    let mut n = 0usize;
    let mut err: Option<IoError> = None;
    while n < min && err.is_none() {
        let (nn, e) = r.read(&mut buf[n..]);
        n += nn;
        err = e;
    }
    if n >= min {
        err = None;
    } else if n > 0 && err == Some(IoError::Eof) {
        err = Some(IoError::UnexpectedEof);
    }
    (n, err)
}

/// Port of `io.ReadFull`.
pub fn read_full(r: &mut dyn Reader, buf: &mut [u8]) -> (usize, Option<IoError>) {
    let n = buf.len();
    read_at_least(r, buf, n)
}

// --------------------------------------------------------------------------
// bytes.Reader / strings.Reader
// --------------------------------------------------------------------------

/// Port of `bytes.NewReader` / `strings.NewReader`.
pub struct BytesReader {
    s: Vec<u8>,
    i: usize,
}

impl BytesReader {
    pub fn new(s: impl Into<Vec<u8>>) -> Self {
        BytesReader { s: s.into(), i: 0 }
    }
}

impl Reader for BytesReader {
    fn read(&mut self, b: &mut [u8]) -> (usize, Option<IoError>) {
        if self.i >= self.s.len() {
            return (0, Some(IoError::Eof));
        }
        let n = std::cmp::min(b.len(), self.s.len() - self.i);
        b[..n].copy_from_slice(&self.s[self.i..self.i + n]);
        self.i += n;
        (n, None)
    }
}

// --------------------------------------------------------------------------
// io.MultiReader
// --------------------------------------------------------------------------

/// Port of `io.MultiReader`.
pub struct MultiReader<'a> {
    readers: Vec<&'a mut dyn Reader>,
    idx: usize,
}

impl<'a> MultiReader<'a> {
    pub fn new(readers: Vec<&'a mut dyn Reader>) -> Self {
        MultiReader { readers, idx: 0 }
    }
}

impl<'a> Reader for MultiReader<'a> {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        while self.idx < self.readers.len() {
            let (n, err) = self.readers[self.idx].read(p);
            let at_eof = err == Some(IoError::Eof);
            if at_eof {
                self.idx += 1;
            }
            if n > 0 || !at_eof {
                // Go suppresses EOF while further readers remain.
                if at_eof && self.idx < self.readers.len() {
                    return (n, None);
                }
                return (n, err);
            }
        }
        (0, Some(IoError::Eof))
    }
}

// --------------------------------------------------------------------------
// bufio.Reader
// --------------------------------------------------------------------------

const DEFAULT_BUF_SIZE: usize = 4096;
const MIN_READ_BUFFER_SIZE: usize = 16;

/// Port of `bufio.Reader`'s state and `Read` path (all that ULID uses).
///
/// The buffering is behaviourally visible: `ulid.Monotonic` wraps its entropy
/// source in a `bufio.Reader`, which reads ahead and therefore changes how many
/// bytes are consumed from the underlying reader.
///
/// The source is passed in per call rather than held as a `&mut` field. Go's
/// `MonotonicEntropy` aliases its entropy source (once through the `bufio.Reader`,
/// once directly through the `rng` interface for the fast path); keeping the
/// source out of this struct is what lets the Rust port reproduce that aliasing
/// without violating borrow rules.
pub struct BufCore {
    buf: Vec<u8>,
    r: usize,
    w: usize,
    err: Option<IoError>,
}

impl BufCore {
    /// Port of `bufio.NewReader`.
    pub fn new() -> Self {
        Self::with_size(DEFAULT_BUF_SIZE)
    }

    /// Port of `bufio.NewReaderSize`.
    pub fn with_size(size: usize) -> Self {
        let size = size.max(MIN_READ_BUFFER_SIZE);
        BufCore {
            buf: vec![0u8; size],
            r: 0,
            w: 0,
            err: None,
        }
    }

    fn read_err(&mut self) -> Option<IoError> {
        self.err.take()
    }

    fn buffered(&self) -> usize {
        self.w - self.r
    }

    /// Port of `(*bufio.Reader).Read`.
    pub fn read_from(&mut self, rd: &mut dyn Reader, p: &mut [u8]) -> (usize, Option<IoError>) {
        if p.is_empty() {
            if self.buffered() > 0 {
                return (0, None);
            }
            return (0, self.read_err());
        }

        if self.r == self.w {
            if self.err.is_some() {
                return (0, self.read_err());
            }
            if p.len() >= self.buf.len() {
                // Large read, empty buffer: read straight into p, bypassing the buffer.
                let (n, e) = rd.read(p);
                self.err = e;
                return (n, self.read_err());
            }
            // One read, filling the buffer.
            self.r = 0;
            self.w = 0;
            let (n, e) = rd.read(&mut self.buf);
            self.err = e;
            if n == 0 {
                return (0, self.read_err());
            }
            self.w += n;
        }

        let n = std::cmp::min(p.len(), self.w - self.r);
        p[..n].copy_from_slice(&self.buf[self.r..self.r + n]);
        self.r += n;
        (n, None)
    }
}

impl Default for BufCore {
    fn default() -> Self {
        Self::new()
    }
}

/// `io.ReadFull` against a `BufCore` + its source (Go: `io.ReadFull(m.Reader, p)`).
pub fn read_full_buffered(
    bc: &mut BufCore,
    rd: &mut dyn Reader,
    buf: &mut [u8],
) -> (usize, Option<IoError>) {
    let min = buf.len();
    let mut n = 0usize;
    let mut err: Option<IoError> = None;
    while n < min && err.is_none() {
        let (nn, e) = bc.read_from(rd, &mut buf[n..]);
        n += nn;
        err = e;
    }
    if n >= min {
        err = None;
    } else if n > 0 && err == Some(IoError::Eof) {
        err = Some(IoError::UnexpectedEof);
    }
    (n, err)
}

// --------------------------------------------------------------------------
// testing/iotest.HalfReader
// --------------------------------------------------------------------------

/// Port of `iotest.HalfReader`: reads half the requested bytes (rounded up).
pub struct HalfReader<'a> {
    r: &'a mut dyn Reader,
}

impl<'a> HalfReader<'a> {
    pub fn new(r: &'a mut dyn Reader) -> Self {
        HalfReader { r }
    }
}

impl<'a> Reader for HalfReader<'a> {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        let n = (p.len() + 1) / 2;
        self.r.read(&mut p[..n])
    }
}

// --------------------------------------------------------------------------
// crypto/rand.Reader
// --------------------------------------------------------------------------

/// Port of `crypto/rand.Reader`, backed by the OS CSPRNG.
///
/// Like Go's `crypto/rand.Reader`, `read` always fills the whole buffer or
/// returns an error — it never reports a short read. On Unix that means
/// looping over `/dev/urandom` and retrying `EINTR`; on Windows it delegates to
/// `BCryptGenRandom`, which fills the buffer in one call.
pub struct CryptoRand {
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(not(unix))]
    _priv: (),
}

impl CryptoRand {
    pub fn new() -> Self {
        #[cfg(unix)]
        {
            use std::fs::File;
            CryptoRand {
                file: File::open("/dev/urandom").expect("ulid: cannot open /dev/urandom"),
            }
        }
        #[cfg(not(unix))]
        {
            CryptoRand { _priv: () }
        }
    }
}

impl Default for CryptoRand {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

#[cfg(windows)]
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(
        h_algorithm: *mut core::ffi::c_void,
        pb_buffer: *mut u8,
        cb_buffer: u32,
        dw_flags: u32,
    ) -> i32;
}

impl Reader for CryptoRand {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        #[cfg(unix)]
        {
            use std::io::ErrorKind;
            use std::io::Read as _;
            let mut n = 0usize;
            while n < p.len() {
                match self.file.read(&mut p[n..]) {
                    Ok(0) => {
                        return (n, Some(IoError::Other("ulid: entropy source EOF".into())));
                    }
                    Ok(k) => n += k,
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return (n, Some(IoError::Other(e.to_string()))),
                }
            }
            (n, None)
        }
        #[cfg(windows)]
        {
            if p.is_empty() {
                return (0, None);
            }
            // BCryptGenRandom fills the entire buffer or fails; NTSTATUS 0 == success.
            let status = unsafe {
                BCryptGenRandom(
                    core::ptr::null_mut(),
                    p.as_mut_ptr(),
                    p.len() as u32,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            };
            if status == 0 {
                (p.len(), None)
            } else {
                (
                    0,
                    Some(IoError::Other(format!(
                        "ulid: BCryptGenRandom failed with status {:#x}",
                        status
                    ))),
                )
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = p;
            (
                0,
                Some(IoError::Other(
                    "ulid: no OS entropy source on this platform".into(),
                )),
            )
        }
    }
}

/// A reader that always yields zero bytes (used by the CLI's `--zero` flag).
pub struct ZeroReader;

impl Reader for ZeroReader {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        for b in p.iter_mut() {
            *b = 0;
        }
        (p.len(), None)
    }
}
