use heapless::String as HString;
use jzf407_logic::web_form::{form_url_decode, html_escape_attr, secret_from_form, FormError};

#[test]
fn form_url_decode_handles_plus_percent_and_utf8() {
    let decoded = form_url_decode::<32>("caf%C3%A9+au+lait%3D1").unwrap();
    assert_eq!(decoded.as_str(), "café au lait=1");
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
}

#[test]
fn html_escape_attr_escapes_every_attribute_breakout_char() {
    let escaped = html_escape_attr::<64>("\"&<>'").unwrap();
    assert_eq!(escaped.as_str(), "&quot;&amp;&lt;&gt;&#39;");
}

#[test]
fn html_escape_attr_fails_closed_on_capacity_overflow() {
    assert_eq!(html_escape_attr::<4>("&&"), Err(FormError::Capacity));
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
