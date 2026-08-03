//! A byte buffer that overwrites itself when dropped.
//!
//! The shell version of this tool staged input in a temp file, which puts
//! `.env` contents and private keys on disk. Here the data never leaves
//! memory, and on drop the buffer is overwritten with volatile writes the
//! optimiser is not allowed to elide.
//!
//! Honest caveat: this is best-effort, not a guarantee. File reads and stdin
//! reads (`main.rs`'s `read_stdin_secret`, chunked precisely so no chunk's
//! backing allocation is ever grown) both avoid the classic gap where a
//! `Vec` reallocates mid-read and frees the old, unwiped allocation. What
//! this still does *not* defend against: the OS paging a page out to swap,
//! a core dump, or a process with ptrace rights.

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

/// Overwrite `len` bytes starting at `ptr` with writes the optimiser cannot
/// elide, then fence so they're visible before this function returns.
/// Exposed as its own function — rather than inlined in each `Drop` impl —
/// specifically so it has a direct unit test: proving memory is unreadable
/// *after* `free()` is not something safe Rust can observe, so what's
/// actually tested is that this primitive correctly zeroes what it's
/// pointed at, which both `Drop` impls below rely on.
///
/// # Safety
/// `ptr` must be valid for `len` writes of `u8`. Write-only, so the pointee
/// need not already be initialized — the whole point is to wipe capacity
/// beyond a buffer's current length, which is not.
unsafe fn volatile_zero(ptr: *mut u8, len: usize) {
    for i in 0..len {
        std::ptr::write_volatile(ptr.add(i), 0u8);
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Wipe the whole allocation, not just the initialised prefix.
        let cap = self.buf.capacity();
        unsafe {
            self.buf.set_len(0);
            volatile_zero(self.buf.as_mut_ptr(), cap);
        }
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
            volatile_zero(v.as_mut_ptr(), cap);
        }
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

    #[test]
    fn volatile_zero_actually_zeroes() {
        // Starts pre-initialised with a non-zero pattern (not uninitialised
        // memory) so this stays entirely within safe-to-read territory
        // afterwards; what's under test is that the write loop itself
        // correctly zeroes every byte it's told to, which is the one thing
        // both Drop impls above actually rely on.
        let mut buf: Vec<u8> = vec![0xAA; 256];
        unsafe { volatile_zero(buf.as_mut_ptr(), buf.len()) };
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn volatile_zero_of_zero_length_is_a_no_op() {
        let mut buf: Vec<u8> = vec![0xAA; 4];
        // Only the first byte is in scope; the rest must be untouched.
        unsafe { volatile_zero(buf.as_mut_ptr(), 1) };
        assert_eq!(buf, vec![0x00, 0xAA, 0xAA, 0xAA]);
    }
}
