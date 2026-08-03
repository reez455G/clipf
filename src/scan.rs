//! Best-effort scan for common secret-token shapes in the payload, so
//! copying an obvious private key or API token gets one visible heads-up
//! instead of landing silently on a clipboard every process in the session
//! can read (D4 stops the payload leaking to *disk*; this is about the
//! actual exposure once it's on the clipboard).
//!
//! Conservative literal-prefix matching only — no regex engine, no entropy
//! heuristics — and it never echoes what it found: only a generic category
//! description, never the match, the line it was on, or its byte offset.
//! Scans at most the first 8 KB of the payload, in place; this never makes
//! a second copy of it.

const SCAN_WINDOW: usize = 8192;

/// One matched secret shape. Fills in "input looks like it contains
/// {description}" for the human-readable warning.
pub struct SecretHit {
    pub description: &'static str,
}

/// Scan the first `SCAN_WINDOW` bytes of `data` for known secret shapes.
/// Every pattern here is a deliberately narrow literal match, chosen to
/// keep false positives on ordinary text near zero — see the module tests
/// for both the true-positive and the "looks similar but isn't" cases each
/// pattern was checked against.
pub fn scan(data: &[u8]) -> Vec<SecretHit> {
    let window = &data[..data.len().min(SCAN_WINDOW)];
    let mut hits = Vec::new();

    // PEM private key: real files always carry both markers, with a key
    // type (RSA/EC/OPENSSH/ENCRYPTED/...) or nothing in between. Requiring
    // both substrings, in either order, is effectively unfakeable by
    // accident.
    if contains(window, b"-----BEGIN") && contains(window, b"PRIVATE KEY-----") {
        hits.push(SecretHit {
            description: "a PEM private key",
        });
    }

    // AWS access key ID: exactly a 4-letter prefix plus 16 uppercase/digit
    // characters, nothing more — matching the *maximal* run (not "at least
    // 16") means a longer, unrelated uppercase-alnum run doesn't false-
    // positive just because it happens to start with AKIA/ASIA.
    if has_prefixed_run(window, b"AKIA", |n| n == 16, is_upper_or_digit)
        || has_prefixed_run(window, b"ASIA", |n| n == 16, is_upper_or_digit)
    {
        hits.push(SecretHit {
            description: "an AWS access key ID",
        });
    }

    // GitHub tokens: ghp_/gho_/ghu_/ghs_/ghr_ (personal/oauth/user-to-
    // server/server-to-server/refresh), each followed by 36+ base62 chars.
    // Classic tokens are exactly 36; fine-grained ones are longer, hence
    // "at least" rather than exact.
    for prefix in [&b"ghp_"[..], b"gho_", b"ghu_", b"ghs_", b"ghr_"] {
        if has_prefixed_run(window, prefix, |n| n >= 36, |b| b.is_ascii_alphanumeric()) {
            hits.push(SecretHit {
                description: "a GitHub access token",
            });
            break;
        }
    }

    if contains(window, b"sk-ant-") {
        hits.push(SecretHit {
            description: "an Anthropic API key",
        });
    }

    for prefix in [&b"xoxa-"[..], b"xoxb-", b"xoxp-", b"xoxr-", b"xoxs-"] {
        if contains(window, prefix) {
            hits.push(SecretHit {
                description: "a Slack token",
            });
            break;
        }
    }

    hits
}

fn is_upper_or_digit(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle).is_some()
}

/// True if `window` contains `prefix` immediately followed by a *maximal*
/// run of bytes satisfying `run_char` whose length satisfies `len_ok` — the
/// run can't be extended by one more matching byte, so e.g. AWS's exactly-
/// 16-char key body can't match inside a longer, unrelated run.
fn has_prefixed_run(
    window: &[u8],
    prefix: &[u8],
    len_ok: impl Fn(usize) -> bool,
    run_char: impl Fn(u8) -> bool,
) -> bool {
    let mut start = 0;
    while let Some(rel) = find(&window[start..], prefix) {
        let pos = start + rel;
        let run_start = pos + prefix.len();
        let mut run_len = 0;
        while run_start + run_len < window.len() && run_char(window[run_start + run_len]) {
            run_len += 1;
        }
        if len_ok(run_len) {
            return true;
        }
        start = pos + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptions(data: &[u8]) -> Vec<&'static str> {
        scan(data).into_iter().map(|h| h.description).collect()
    }

    // --- true positives -----------------------------------------------

    #[test]
    fn detects_pem_private_key() {
        let key = b"-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQ==\n-----END RSA PRIVATE KEY-----\n";
        assert_eq!(descriptions(key), vec!["a PEM private key"]);
    }

    #[test]
    fn detects_pem_private_key_with_no_type_name() {
        assert_eq!(
            descriptions(b"-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"),
            vec!["a PEM private key"]
        );
    }

    #[test]
    fn detects_aws_access_key() {
        assert_eq!(descriptions(b"AKIAIOSFODNN7EXAMPLE"), vec!["an AWS access key ID"]);
        assert_eq!(
            descriptions(b"key=ASIAABCDEFGHIJ123456 rest"),
            vec!["an AWS access key ID"]
        );
    }

    #[test]
    fn detects_github_token() {
        let tok = "ghp_".to_string() + &"a".repeat(36);
        assert_eq!(descriptions(tok.as_bytes()), vec!["a GitHub access token"]);
        let long_tok = "gho_".to_string() + &"B3x9".repeat(20); // 80 chars, fine-grained-length
        assert_eq!(descriptions(long_tok.as_bytes()), vec!["a GitHub access token"]);
    }

    #[test]
    fn detects_anthropic_key() {
        assert_eq!(
            descriptions(b"ANTHROPIC_API_KEY=sk-ant-api03-abc123xyz"),
            vec!["an Anthropic API key"]
        );
    }

    #[test]
    fn detects_slack_token() {
        assert_eq!(descriptions(b"xoxb-1234567890-abcdefg"), vec!["a Slack token"]);
        assert_eq!(descriptions(b"xoxp-0000000000-1111111111"), vec!["a Slack token"]);
    }

    #[test]
    fn multiple_distinct_hits_are_all_reported() {
        let mixed = format!(
            "AKIAIOSFODNN7EXAMPLE and sk-ant-abcdef and ghp_{}",
            "x".repeat(36)
        );
        let mut ds = descriptions(mixed.as_bytes());
        ds.sort_unstable();
        assert_eq!(
            ds,
            vec!["a GitHub access token", "an AWS access key ID", "an Anthropic API key"]
        );
    }

    // --- false positives: ordinary content that must NOT match ---------

    #[test]
    fn ordinary_prose_is_clean() {
        let text = b"Dear team, please review the AKIA presentation deck and \
                      the sk-ant guidelines before Friday's standup. Thanks!";
        assert!(descriptions(text).is_empty(), "{:?}", descriptions(text));
    }

    #[test]
    fn short_prefix_without_a_real_body_is_clean() {
        // "AKIA" not followed by a full 16-char uppercase/digit run.
        assert!(descriptions(b"AKIA is a term you might see in logs").is_empty());
        // Prefix present but body too short.
        assert!(descriptions(b"AKIAABC123").is_empty());
        // gh_ prefixes without a 36-char run.
        assert!(descriptions(b"ghp_short").is_empty());
        assert!(descriptions(b"gho_only20charslongxx").is_empty());
    }

    #[test]
    fn aws_run_longer_than_sixteen_does_not_match() {
        // A maximal run of 20 upper/digit chars after AKIA is not a valid
        // (fixed-width) AWS key and should not match, by design.
        assert!(descriptions(b"AKIAAAAAAAAAAAAAAAAAAAAA").is_empty());
    }

    #[test]
    fn ordinary_uppercase_identifiers_are_clean() {
        assert!(descriptions(b"CONST_MAX_RETRY_COUNT_1234567890AB = 5").is_empty());
    }

    #[test]
    fn json_config_without_real_secrets_is_clean() {
        let cfg = br#"{"region":"us-east-1","role":"admin","xox_flag":false}"#;
        assert!(descriptions(cfg).is_empty());
    }

    #[test]
    fn plain_hex_and_base64_blobs_are_clean() {
        let hex = b"deadbeefcafebabe0123456789abcdef0123456789abcdef0123456789abcd";
        assert!(descriptions(hex).is_empty());
    }

    #[test]
    fn empty_input_is_clean() {
        assert!(descriptions(b"").is_empty());
    }

    // --- window bound ----------------------------------------------------

    #[test]
    fn scan_window_is_bounded_to_8kb() {
        // Pattern starts exactly at the boundary -> entirely outside the
        // scanned window -> not found. The scan is a hard prefix truncation,
        // not a boundary-aware search, so this is the documented limit of
        // "best effort", not a bug.
        let mut beyond = vec![b'x'; SCAN_WINDOW];
        beyond.extend_from_slice(b"AKIAIOSFODNN7EXAMPLE");
        assert!(descriptions(&beyond).is_empty());

        // Pattern starts with just enough room to finish exactly at the
        // boundary -> fully inside the window -> found.
        let mut fits = vec![b'x'; SCAN_WINDOW - 20];
        fits.extend_from_slice(b"AKIAIOSFODNN7EXAMPLE");
        assert_eq!(fits.len(), SCAN_WINDOW);
        assert_eq!(descriptions(&fits), vec!["an AWS access key ID"]);

        // Pattern starts inside the window but is truncated mid-match ->
        // the run never reaches the required 16 chars -> not found.
        let mut truncated = vec![b'x'; SCAN_WINDOW - 5];
        truncated.extend_from_slice(b"AKIAIOSFODNN7EXAMPLE");
        assert!(descriptions(&truncated).is_empty());
    }
}
