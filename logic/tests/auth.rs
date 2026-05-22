// Native unit tests for HTTP Basic Auth token construction.
// Run with: cargo test --test auth

use jzf407_logic::auth::basic_token;

#[test]
fn known_vector() {
    // base64("admin:pass") — verified against `printf 'admin:pass' | base64`.
    assert_eq!(basic_token("admin", "pass").as_str(), "YWRtaW46cGFzcw==");
}

#[test]
fn one_pad_byte() {
    // "user:pw" is 7 bytes -> 1 padding char.
    assert_eq!(basic_token("user", "pw").as_str(), "dXNlcjpwdw==");
}

#[test]
fn rfc7617_vector() {
    // The canonical RFC 7617 example: base64("Aladdin:open sesame").
    assert_eq!(
        basic_token("Aladdin", "open sesame").as_str(),
        "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
    );
}

#[test]
fn empty_user_and_pass() {
    // Just the ":" separator -> base64(":") = "Og==".
    assert_eq!(basic_token("", "").as_str(), "Og==");
}

#[test]
fn empty_user_only() {
    // base64(":pass")
    assert_eq!(basic_token("", "pass").as_str(), "OnBhc3M=");
}
