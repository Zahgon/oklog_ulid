//! Shared helpers for the ported test suites.
#![allow(dead_code)]

use std::panic;
use ulid::{Error, GoRand, Reader, ULID};

/// Stand-in for Go's `testing/quick` value generator.
///
/// Go seeds from the clock; we seed deterministically so any failure is
/// reproducible. Iteration counts match Go's `MaxCount` exactly.
pub struct Quick {
    r: GoRand,
}

impl Quick {
    pub fn new(seed: i64) -> Self {
        Quick {
            r: GoRand::new(seed),
        }
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        self.r.read(buf);
    }

    pub fn ulid(&mut self) -> ULID {
        let mut b = [0u8; 16];
        self.fill(&mut b);
        ULID(b)
    }

    pub fn arr10(&mut self) -> [u8; 10] {
        let mut b = [0u8; 10];
        self.fill(&mut b);
        b
    }

    pub fn arr26(&mut self) -> [u8; 26] {
        let mut b = [0u8; 26];
        self.fill(&mut b);
        b
    }

    pub fn u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill(&mut b);
        u64::from_le_bytes(b)
    }
}

/// Go's default `quick.Config.MaxCount`.
pub const QUICK_DEFAULT: usize = 100;

/// Recovers a panic payload as a string, mirroring Go's `recover()`.
pub fn recover_msg<F: FnOnce() + panic::UnwindSafe>(f: F) -> Option<String> {
    let prev = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let res = panic::catch_unwind(f);
    panic::set_hook(prev);
    match res {
        Ok(()) => None,
        Err(e) => {
            if let Some(s) = e.downcast_ref::<String>() {
                Some(s.clone())
            } else if let Some(s) = e.downcast_ref::<&str>() {
                Some((*s).to_string())
            } else {
                Some("<non-string panic>".to_string())
            }
        }
    }
}

/// Renders an `Option<Error>` the way Go's `fmt.Sprint(err)` does.
pub fn sprint_err(e: &Option<Error>) -> String {
    match e {
        None => "<nil>".to_string(),
        Some(e) => e.to_string(),
    }
}
