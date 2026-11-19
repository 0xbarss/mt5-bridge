/// Maximum length of an MT5 order/position comment in bytes.
pub const MT5_COMMENT_MAX_BYTES: usize = 31;

/// Length in characters of the fixed-width wire token derived from a `client_order_id`.
pub const WIRE_ID_LEN: usize = 13;

/// Maximum accepted length of a caller-supplied `client_order_id`, in bytes.
///
/// The full ID is never sent over the wire (only its [`wire_id`] is), so this limit exists
/// purely to keep persisted state and log lines sane.
pub const MAX_CLIENT_ORDER_ID_BYTES: usize = 128;

/// Reserved comment prefix used to embed the wire token in MT5 comments.
pub const WIRE_COMMENT_PREFIX: &str = "cid:";

const CROCKFORD32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Truncate `s` to at most `max_bytes` bytes **without splitting a UTF-8 character**.
///
/// Unlike a raw `&s[..n]` slice this can never panic, no matter what the caller supplied.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Derive the short, deterministic wire token for a `client_order_id`.
///
/// MT5 comments are limited to 31 bytes, so the full client order ID cannot be embedded
/// verbatim without truncation (which would make IDs sharing a prefix indistinguishable).
/// Instead the *entire* ID is hashed to 64 bits (FNV-1a with a murmur3 `fmix64` finalizer)
/// and rendered as [`WIRE_ID_LEN`] Crockford-base32 characters, e.g. `"3K9Q0MZ7V1XTB"`.
///
/// **Stability contract:** this function is part of the persisted/wire format. Changing it
/// would orphan every in-flight order across an upgrade, so it is pinned by test vectors.
pub fn wire_id(client_order_id: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in client_order_id.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // murmur3 fmix64 avalanche so structurally similar IDs diverge across all bits
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;

    let mut out = String::with_capacity(WIRE_ID_LEN);
    for i in 0..WIRE_ID_LEN {
        let shift = 60 - 5 * i as u32;
        out.push(CROCKFORD32[((h >> shift) & 0x1f) as usize] as char);
    }
    out
}

fn is_wire_token(s: &str) -> bool {
    s.len() == WIRE_ID_LEN && s.bytes().all(|b| CROCKFORD32.contains(&b))
}

/// Extract the wire token from an MT5 comment produced by [`OrderRequest::effective_comment`](crate::types::order_request::OrderRequest::effective_comment).
///
/// Returns `Some(token)` only for a well-formed `cid:<13 base32 chars>` prefix that is
/// followed by the end of the string or a non-alphanumeric separator (brokers sometimes
/// append suffixes such as `[sl 1.0850]` on closing deals). Free-text comments, including
/// ones that merely start with `cid:`, return `None`.
pub fn parse_wire_id(comment: &str) -> Option<&str> {
    let rest = comment.strip_prefix(WIRE_COMMENT_PREFIX)?;
    let token = rest.get(..WIRE_ID_LEN)?;
    if !is_wire_token(token) {
        return None;
    }
    match rest[WIRE_ID_LEN..].chars().next() {
        None => Some(token),
        Some(c) if !c.is_ascii_alphanumeric() => Some(token),
        Some(_) => None,
    }
}

/// Returns `true` if `comment` carries the wire token belonging to `client_order_id`.
pub fn comment_matches_client_order_id(comment: &str, client_order_id: &str) -> bool {
    match parse_wire_id(comment) {
        Some(token) => token == wire_id(client_order_id),
        None => false,
    }
}
