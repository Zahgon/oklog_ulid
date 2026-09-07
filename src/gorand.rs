//! Bit-exact port of Go's `math/rand` seeded generator (`rngSource` + `Rand`).
//!
//! This is required, not cosmetic: the upstream Go test-suite contains an
//! `Example` that asserts an exact ULID string produced from a fixed seed
//! (`0000XSNJG0MQJHBF4QX1EFD6Y3`), and several tests seed `math/rand`
//! deterministically. Reproducing those outputs requires the identical
//! Additive Lagged Fibonacci generator, including Go's `rngCooked` seed table.
//!
//! Ported from $GOROOT/src/math/rand/rng.go and rand.go.

use crate::goio::{IoError, Reader};
use crate::rng_cooked::RNG_COOKED;

const RNG_LEN: usize = 607;
const RNG_TAP: usize = 273;
const RNG_MASK: i64 = i64::MAX; // 1<<63 - 1
const INT32MAX: i32 = i32::MAX; // (1 << 31) - 1

// seedrand: x = (16807 * x) mod (2^31 - 1), via Schrage's method.
// Go uses A=48271, Q=44488, R=3399.
const A: i32 = 48271;
const Q: i32 = 44488;
const R: i32 = 3399;

fn seedrand(x: i32) -> i32 {
    let hi = x / Q;
    let lo = x % Q;
    // Go relies on int32 wrapping semantics here.
    let mut x = A.wrapping_mul(lo).wrapping_sub(R.wrapping_mul(hi));
    if x < 0 {
        x = x.wrapping_add(INT32MAX);
    }
    x
}

/// Port of Go's `rand.rngSource`.
pub struct RngSource {
    tap: usize,
    feed: usize,
    vec: [i64; RNG_LEN],
}

impl RngSource {
    /// Equivalent to Go's `rand.NewSource(seed)`.
    pub fn new(seed: i64) -> Self {
        let mut s = RngSource {
            tap: 0,
            feed: RNG_LEN - RNG_TAP,
            vec: [0i64; RNG_LEN],
        };
        s.seed(seed);
        s
    }

    /// Port of `(*rngSource).Seed`.
    pub fn seed(&mut self, seed: i64) {
        self.tap = 0;
        self.feed = RNG_LEN - RNG_TAP;

        let mut seed = seed % INT32MAX as i64;
        if seed < 0 {
            seed += INT32MAX as i64;
        }
        if seed == 0 {
            seed = 89482311;
        }

        let mut x = seed as i32;
        // Go iterates i from -20 so the generator is warmed up before filling vec.
        let mut i: i64 = -20;
        while i < RNG_LEN as i64 {
            x = seedrand(x);
            if i >= 0 {
                let mut u: i64 = (x as i64) << 40;
                x = seedrand(x);
                u ^= (x as i64) << 20;
                x = seedrand(x);
                u ^= x as i64;
                u ^= RNG_COOKED[i as usize];
                self.vec[i as usize] = u;
            }
            i += 1;
        }
    }

    /// Port of `(*rngSource).Uint64`.
    pub fn uint64(&mut self) -> u64 {
        if self.tap == 0 {
            self.tap = RNG_LEN - 1;
        } else {
            self.tap -= 1;
        }
        if self.feed == 0 {
            self.feed = RNG_LEN - 1;
        } else {
            self.feed -= 1;
        }
        // Go: x := rng.vec[rng.feed] + rng.vec[rng.tap] (int64 wraparound)
        let x = self.vec[self.feed].wrapping_add(self.vec[self.tap]);
        self.vec[self.feed] = x;
        x as u64
    }

    /// Port of `(*rngSource).Int63`.
    pub fn int63(&mut self) -> i64 {
        (self.uint64() & (RNG_MASK as u64)) as i64
    }
}

/// Port of Go's `rand.Rand` (the parts the ULID package exercises).
///
/// Mirrors `Int63`, `Int63n` and the `io.Reader` implementation, including the
/// `readVal`/`readPos` byte-buffering that makes `Read` stateful across calls.
pub struct GoRand {
    src: RngSource,
    read_val: i64,
    read_pos: i8,
}

impl GoRand {
    /// Equivalent to Go's `rand.New(rand.NewSource(seed))`.
    pub fn new(seed: i64) -> Self {
        GoRand {
            src: RngSource::new(seed),
            read_val: 0,
            read_pos: 0,
        }
    }

    pub fn int63(&mut self) -> i64 {
        self.src.int63()
    }

    /// Port of `(*Rand).Int63n`. Panics for n <= 0, exactly like Go.
    pub fn int63n(&mut self, n: i64) -> i64 {
        if n <= 0 {
            panic!("invalid argument to Int63n");
        }
        if n & (n - 1) == 0 {
            // n is a power of two, can mask
            return self.int63() & (n - 1);
        }
        let max = (((1u64 << 63) - 1) - (1u64 << 63) % (n as u64)) as i64;
        let mut v = self.int63();
        while v > max {
            v = self.int63();
        }
        v % n
    }
}

/// Port of Go's `read(p, src, readVal, readPos)` used by `(*Rand).Read`.
impl Reader for GoRand {
    fn read(&mut self, p: &mut [u8]) -> (usize, Option<IoError>) {
        let mut pos = self.read_pos;
        let mut val = self.read_val;
        let mut n = 0usize;
        while n < p.len() {
            if pos == 0 {
                val = self.src.int63();
                pos = 7;
            }
            p[n] = val as u8;
            val >>= 8;
            pos -= 1;
            n += 1;
        }
        self.read_pos = pos;
        self.read_val = val;
        (n, None)
    }
}

/// Marker for Go's unexported `rng interface{ Int63n(n int64) int64 }`.
///
/// `ulid.Monotonic` type-asserts its entropy source against this interface to
/// take a faster path; only `*rand.Rand` satisfies it.
pub trait Int63nSource {
    fn int63n(&mut self, n: i64) -> i64;
}

impl Int63nSource for GoRand {
    fn int63n(&mut self, n: i64) -> i64 {
        GoRand::int63n(self, n)
    }
}
