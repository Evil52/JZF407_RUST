//! HTTP Basic Auth token construction (pure, host-testable).
//!
//! The firmware never base64-*decodes* an incoming `Authorization` header.
//! Instead it builds the *expected* `base64(user:pass)` once at startup and
//! string-compares it to the token the browser sends after `Basic `. That is
//! far less code than a decoder, needs no allocation, and keeps the comparison
//! on a known-length string.
//!
//! With `user` and `pass` each ≤ 32 bytes the joined `user:pass` is ≤ 65 bytes,
//! which base64-encodes to ≤ 88 characters — hence the `String<96>` result.

use heapless::String;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Build the base64 of `user:pass` — i.e. the value that follows `Basic ` in an
/// HTTP `Authorization` header. Comparing this to the incoming token is how the
/// web server authenticates a request without decoding anything.
pub fn basic_token(user: &str, pass: &str) -> String<96> {
    // Join "user:pass" into a scratch buffer (≤ 65 bytes for ≤32+1+32).
    let mut raw: heapless::Vec<u8, 96> = heapless::Vec::new();
    let _ = raw.extend_from_slice(user.as_bytes());
    let _ = raw.push(b':');
    let _ = raw.extend_from_slice(pass.as_bytes());

    let mut out: String<96> = String::new();
    for chunk in raw.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        let _ = out.push(B64[(b0 >> 2) as usize] as char);
        let _ = out.push(B64[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            let _ = out.push(B64[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            let _ = out.push('=');
        }
        if chunk.len() > 2 {
            let _ = out.push(B64[(b2 & 0x3f) as usize] as char);
        } else {
            let _ = out.push('=');
        }
    }
    out
}
