//! Minimal RFC 4648 base64. Exists so the crate has zero dependencies.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encoded length for `n` input bytes, including padding.
pub const fn encoded_len(n: usize) -> usize {
    (n + 2) / 3 * 4
}

pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(encoded_len(input.len()));

    let mut chunks = input.chunks_exact(3);
    for c in &mut chunks {
        let n = u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
        out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n >> 6 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
    }

    match chunks.remainder() {
        [a] => {
            let n = u32::from(*a) << 16;
            out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
            out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        [a, b] => {
            let n = u32::from(*a) << 16 | u32::from(*b) << 8;
            out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
            out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
            out.push(ALPHABET[(n >> 6 & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }

    out
}

/// Decoder, used by the test suite to prove round-trip fidelity.
#[cfg(test)]
pub fn decode(input: &str) -> Result<Vec<u8>, &'static str> {
    fn val(b: u8) -> Result<u32, &'static str> {
        match b {
            b'A'..=b'Z' => Ok(u32::from(b - b'A')),
            b'a'..=b'z' => Ok(u32::from(b - b'a') + 26),
            b'0'..=b'9' => Ok(u32::from(b - b'0') + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("invalid base64 character"),
        }
    }

    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if bytes.len() % 4 != 0 {
        return Err("base64 length is not a multiple of 4");
    }

    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for quad in bytes.chunks_exact(4) {
        let pad = quad.iter().filter(|&&b| b == b'=').count();
        let mut n = 0u32;
        for (i, &b) in quad.iter().enumerate() {
            n |= if b == b'=' { 0 } else { val(b)? } << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encoded_len_matches_encode() {
        for n in 0..200 {
            let data = vec![0xa5u8; n];
            assert_eq!(encoded_len(n), encode(&data).len(), "n={n}");
        }
    }

    #[test]
    fn round_trips_all_byte_values() {
        let data: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&data)).unwrap(), data);
    }

    #[test]
    fn round_trips_every_length_up_to_1k() {
        for n in 0..1024 {
            let data: Vec<u8> = (0..n).map(|i| (i * 31 % 256) as u8).collect();
            assert_eq!(decode(&encode(&data)).unwrap(), data, "n={n}");
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode("Zm9vYmFy=").is_err());
        assert!(decode("Zm9!").is_err());
    }
}
