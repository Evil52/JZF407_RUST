//! Builds the expected base64(user:pass) once at startup; incoming credentials
//! are compared against it — nothing is ever base64-decoded at runtime.

use heapless::String;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn basic_token(user: &str, pass: &str) -> String<96> {
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
