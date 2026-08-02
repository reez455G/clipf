//! OSC 52 escape sequence construction.
//!
//! OSC 52 is the mechanism that makes remote copying work at all: the sequence
//! travels up the existing SSH connection and the *local* terminal emulator
//! puts the payload in the local clipboard. Nothing needs to be installed on
//! the remote host.

use crate::secret::Secret;
use crate::term::Multiplexer;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// GNU screen truncates long DCS passthrough strings, so the payload has to be
/// split across several of them. screen concatenates the contents, so the
/// receiving terminal still sees one continuous OSC 52 sequence. 448 leaves
/// room under the historical 512-byte limit.
const SCREEN_CHUNK: usize = 448;

pub struct Sequence {
    pub bytes: Secret,
}

impl Sequence {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
}

/// Build the full sequence for `b64`, wrapped as needed for the multiplexer.
///
/// `passthrough` forces the tmux DCS wrapper. Without it, tmux is expected to
/// intercept the plain sequence itself, which is what `set-clipboard on` does.
pub fn build(b64: &str, mux: Multiplexer, passthrough: bool) -> Sequence {
    let inner_len = 7 + b64.len() + 1; // ESC ] 5 2 ; c ; ... BEL
    let mut inner = Vec::with_capacity(inner_len);
    inner.push(ESC);
    inner.extend_from_slice(b"]52;c;");
    inner.extend_from_slice(b64.as_bytes());
    inner.push(BEL);

    let out = match mux {
        Multiplexer::Tmux if passthrough => {
            // ESC P tmux ; <inner, every ESC doubled> ESC \
            let mut v = Vec::with_capacity(inner.len() * 2 + 16);
            v.push(ESC);
            v.extend_from_slice(b"Ptmux;");
            for &b in &inner {
                if b == ESC {
                    v.push(ESC);
                }
                v.push(b);
            }
            v.push(ESC);
            v.push(b'\\');
            v
        }
        Multiplexer::Screen => {
            let mut v = Vec::with_capacity(inner.len() + inner.len() / SCREEN_CHUNK * 4 + 8);
            for chunk in inner.chunks(SCREEN_CHUNK) {
                v.push(ESC);
                v.push(b'P');
                v.extend_from_slice(chunk);
                v.push(ESC);
                v.push(b'\\');
            }
            v
        }
        _ => inner,
    };

    Sequence {
        bytes: Secret::from_vec(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_sequence_shape() {
        let s = build("Zm9v", Multiplexer::None, false);
        assert_eq!(&*s.bytes, b"\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn tmux_without_passthrough_is_plain() {
        let s = build("Zm9v", Multiplexer::Tmux, false);
        assert_eq!(&*s.bytes, b"\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn tmux_passthrough_doubles_inner_esc() {
        let s = build("Zm9v", Multiplexer::Tmux, true);
        assert_eq!(&*s.bytes, b"\x1bPtmux;\x1b\x1b]52;c;Zm9v\x07\x1b\\");
    }

    #[test]
    fn screen_wraps_short_payload_in_one_chunk() {
        let s = build("Zm9v", Multiplexer::Screen, false);
        assert_eq!(&*s.bytes, b"\x1bP\x1b]52;c;Zm9v\x07\x1b\\");
    }

    #[test]
    fn screen_splits_long_payload_and_preserves_inner_stream() {
        let b64 = "A".repeat(2000);
        let s = build(&b64, Multiplexer::Screen, false);

        // Strip every ESC P ... ESC \ wrapper and the inner stream must come
        // back byte-for-byte identical to the unwrapped sequence.
        let mut recovered = Vec::new();
        let bytes: &[u8] = &s.bytes;
        let mut i = 0;
        let mut chunks = 0;
        while i < bytes.len() {
            assert_eq!(bytes[i], ESC);
            assert_eq!(bytes[i + 1], b'P');
            i += 2;
            let start = i;
            while !(bytes[i] == ESC && bytes[i + 1] == b'\\') {
                i += 1;
            }
            recovered.extend_from_slice(&bytes[start..i]);
            i += 2;
            chunks += 1;
        }
        assert!(chunks > 1, "expected multiple chunks, got {chunks}");
        let expected = build(&b64, Multiplexer::None, false);
        assert_eq!(recovered, &*expected.bytes);
    }

    #[test]
    fn empty_payload_clears_the_clipboard() {
        let s = build("", Multiplexer::None, false);
        assert_eq!(&*s.bytes, b"\x1b]52;c;\x07");
    }
}
