use heapless::String as HString;
use jzf407_logic::web_form::{form_url_decode, html_escape_attr, secret_from_form, FormError};

#[test]
fn form_url_decode_handles_plus_percent_and_utf8() {
    let decoded = form_url_decode::<32>("caf%C3%A9+au+lait%3D1").unwrap();
    assert_eq!(decoded.as_str(), "café au lait=1");
}

#[test]
fn form_url_decode_accepts_lowercase_hex_digits() {
    let decoded = form_url_decode::<8>("%c3%a9").unwrap();
    assert_eq!(decoded.as_str(), "é");
}

#[test]
fn form_url_decode_rejects_invalid_percent_escape() {
    assert_eq!(
        form_url_decode::<16>("abc%zz"),
        Err(FormError::InvalidEncoding)
    );
    assert_eq!(
        form_url_decode::<16>("abc%"),
        Err(FormError::InvalidEncoding)
    );
}

#[test]
fn form_url_decode_rejects_invalid_utf8_after_percent_decode() {
    assert_eq!(
        form_url_decode::<16>("bad%D0"),
        Err(FormError::InvalidEncoding)
    );
}

#[test]
fn form_url_decode_rejects_capacity_overflow_instead_of_truncating() {
    assert_eq!(form_url_decode::<4>("12345"), Err(FormError::Capacity));
    assert_eq!(form_url_decode::<0>("+"), Err(FormError::Capacity));
    assert_eq!(form_url_decode::<0>("%41"), Err(FormError::Capacity));
}

#[test]
fn form_url_decode_rejects_nul_that_eeprom_would_truncate() {
    assert_eq!(
        form_url_decode::<16>("admin%00ignored"),
        Err(FormError::InvalidEncoding)
    );
    assert_eq!(
        form_url_decode::<16>("admin\0ignored"),
        Err(FormError::InvalidEncoding)
    );
}

#[test]
fn html_escape_attr_escapes_every_attribute_breakout_char() {
    let escaped = html_escape_attr::<64>("\"&<>'").unwrap();
    assert_eq!(escaped.as_str(), "&quot;&amp;&lt;&gt;&#39;");
}

#[test]
fn html_escape_attr_fails_closed_on_capacity_overflow() {
    assert_eq!(html_escape_attr::<4>("&&"), Err(FormError::Capacity));
    assert_eq!(html_escape_attr::<3>("abcd"), Err(FormError::Capacity));
}

#[test]
fn html_escape_attr_preserves_safe_text_and_unicode() {
    let escaped = html_escape_attr::<32>("node-01 Привет").unwrap();
    assert_eq!(escaped.as_str(), "node-01 Привет");
}

#[test]
fn empty_secret_form_field_preserves_existing_secret() {
    let current = HString::<32>::try_from("old-secret").unwrap();
    let merged = secret_from_form(&current, "").unwrap();
    assert_eq!(merged.as_str(), "old-secret");
}

#[test]
fn non_empty_secret_form_field_replaces_existing_secret() {
    let current = HString::<32>::try_from("old-secret").unwrap();
    let merged = secret_from_form(&current, "new-secret").unwrap();
    assert_eq!(merged.as_str(), "new-secret");
}

#[test]
fn oversized_secret_form_field_is_rejected() {
    let current = HString::<32>::try_from("old-secret").unwrap();
    let too_long = "x".repeat(33);
    assert_eq!(
        secret_from_form(&current, &too_long),
        Err(FormError::Capacity)
    );
}
