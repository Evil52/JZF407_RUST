//! Form url-decode / HTML-escape helpers. Fail closed: bad percent encoding or
//! capacity overflow errors out instead of silently truncating credentials.

use heapless::{String, Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormError {
    Capacity,
    InvalidEncoding,
}

pub fn form_url_decode<const N: usize>(s: &str) -> Result<String<N>, FormError> {
    let mut bytes_out: Vec<u8, N> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                bytes_out.push(b' ').map_err(|_| FormError::Capacity)?;
                i += 1;
            }
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err(FormError::InvalidEncoding);
                }
                let h = hex_digit(bytes[i + 1]).ok_or(FormError::InvalidEncoding)?;
                let l = hex_digit(bytes[i + 2]).ok_or(FormError::InvalidEncoding)?;
                bytes_out
                    .push((h << 4) | l)
                    .map_err(|_| FormError::Capacity)?;
                i += 3;
            }
            byte => {
                bytes_out.push(byte).map_err(|_| FormError::Capacity)?;
                i += 1;
            }
        }
    }

    let decoded =
        core::str::from_utf8(bytes_out.as_slice()).map_err(|_| FormError::InvalidEncoding)?;
    String::try_from(decoded).map_err(|_| FormError::Capacity)
}

pub fn html_escape_attr<const N: usize>(s: &str) -> Result<String<N>, FormError> {
    let mut out = String::<N>::new();
    for ch in s.chars() {
        match ch {
            '&' => push_str(&mut out, "&amp;")?,
            '<' => push_str(&mut out, "&lt;")?,
            '>' => push_str(&mut out, "&gt;")?,
            '"' => push_str(&mut out, "&quot;")?,
            '\'' => push_str(&mut out, "&#39;")?,
            _ => out.push(ch).map_err(|_| FormError::Capacity)?,
        }
    }
    Ok(out)
}

/// Empty submitted value means "keep the stored secret" — passwords are never
/// rendered back into the page, so an untouched field must not wipe them.
pub fn secret_from_form<const N: usize>(
    current: &String<N>,
    submitted: &str,
) -> Result<String<N>, FormError> {
    if submitted.is_empty() {
        Ok(current.clone())
    } else {
        String::try_from(submitted).map_err(|_| FormError::Capacity)
    }
}

fn push_str<const N: usize>(out: &mut String<N>, s: &str) -> Result<(), FormError> {
    out.push_str(s).map_err(|_| FormError::Capacity)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
