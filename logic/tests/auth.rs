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


#[test]
fn max_configured_credentials_fit_token_buffer_without_truncation() {
    let user = "u".repeat(jzf407_logic::config::CRED_MAX);
    let pass = "p".repeat(jzf407_logic::config::CRED_MAX);
    let token = basic_token(&user, &pass);

    let raw_len = jzf407_logic::config::CRED_MAX * 2 + 1; // user ':' pass
    let expected_base64_len = ((raw_len + 2) / 3) * 4;
    assert_eq!(expected_base64_len, 88);
    assert_eq!(token.len(), expected_base64_len);
    assert!(token.as_str().ends_with('='));
}
