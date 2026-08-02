//! A byte buffer that overwrites itself when dropped.
//!
//! The shell version of this tool staged input in a temp file, which puts
//! `.env` contents and private keys on disk. Here the data never leaves
//! memory, and on drop the buffer is overwritten with volatile writes the
//! optimiser is not allowed to elide.
//!
//! Honest caveat: this is best-effort. If the `Vec` reallocates while growing,
//! the old allocation is freed without being wiped. Reading from a file
//! preallocates the exact size so no reallocation happens; reading from stdin
//! has unknown length, so growth (and therefore stale copies) is possible.
//! Nothing here defends against swap, core dumps, or a hostile process with
//! ptrace rights.

use std::ops::Deref;

pub struct Secret {
    buf: Vec<u8>,
}

impl Secret {
    pub fn from_vec(buf: Vec<u8>) -> Self {
        Self { buf }
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            buf: Vec::with_capacity(n),
        }
    }

    pub fn as_mut_vec(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Drop trailing CR/LF bytes. Useful for tokens, hashes and IPs, where a
    /// stray newline breaks whatever you paste into.
    pub fn trim_trailing_newlines(&mut self) {
        while matches!(self.buf.last(), Some(b'\n') | Some(b'\r')) {
            self.buf.pop();
        }
    }
}

impl Deref for Secret {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.buf
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Wipe the whole allocation, not just the initialised prefix.
        let cap = self.buf.capacity();
        unsafe {
            self.buf.set_len(0);
            let p = self.buf.as_mut_ptr();
            for i in 0..cap {
                std::ptr::write_volatile(p.add(i), 0u8);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

/// Same treatment for the base64 form, which is just as sensitive.
pub struct SecretString(pub String);

impl Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        let cap = self.0.capacity();
        unsafe {
            let v = self.0.as_mut_vec();
            v.set_len(0);
            let p = v.as_mut_ptr();
            for i in 0..cap {
                std::ptr::write_volatile(p.add(i), 0u8);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_only_trailing_newlines() {
        let mut s = Secret::from_vec(b"a\nb\n\r\n".to_vec());
        s.trim_trailing_newlines();
        assert_eq!(&*s, b"a\nb");
    }

    #[test]
    fn trimming_empty_is_safe() {
        let mut s = Secret::from_vec(b"\n\n\n".to_vec());
        s.trim_trailing_newlines();
        assert!(s.is_empty());
    }

    #[test]
    fn deref_exposes_bytes() {
        let s = Secret::from_vec(vec![1, 2, 3]);
        assert_eq!(s.len(), 3);
        assert_eq!(&*s, &[1, 2, 3]);
    }
}
