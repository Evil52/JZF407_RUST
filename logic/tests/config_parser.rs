// Native unit tests for config parsing and serialisation.
// Run with: cargo test --test config_parser

use jzf407_logic::config::{parse_ipv4, parse_port, NetworkConfig};

// ---- parse_ipv4 ----

#[test] fn valid_ip()       { assert_eq!(parse_ipv4("192.168.1.1"), Some([192,168,1,1])); }
#[test] fn all_zeros()      { assert_eq!(parse_ipv4("0.0.0.0"),     Some([0,0,0,0])); }
#[test] fn broadcast()      { assert_eq!(parse_ipv4("255.255.255.255"), Some([255,255,255,255])); }
#[test] fn too_few_octets() { assert_eq!(parse_ipv4("192.168.1"),  None); }
#[test] fn too_many_octets(){ assert_eq!(parse_ipv4("1.2.3.4.5"),  None); }
#[test] fn octet_overflow() { assert_eq!(parse_ipv4("256.0.0.1"),  None); }
#[test] fn negative_octet() { assert_eq!(parse_ipv4("-1.0.0.1"),   None); }
#[test] fn empty_string()   { assert_eq!(parse_ipv4(""),            None); }
#[test] fn alpha_in_ip()    { assert_eq!(parse_ipv4("192.168.x.1"), None); }

// ---- parse_port ----

#[test] fn valid_port()   { assert_eq!(parse_port("1883"), Some(1883)); }
#[test] fn max_port()     { assert_eq!(parse_port("65535"), Some(65535)); }
#[test] fn zero_port()    { assert_eq!(parse_port("0"),    None); }
#[test] fn overflow_port(){ assert_eq!(parse_port("65536"), None); }
#[test] fn alpha_port()   { assert_eq!(parse_port("abc"),  None); }
#[test] fn port_spaces()  { assert_eq!(parse_port(" 1883 "), Some(1883)); }

// ---- NetworkConfig round-trip ----

#[test]
fn default_config_round_trip() {
    let cfg = NetworkConfig::default();
    let bytes = cfg.to_bytes();
    let restored = NetworkConfig::from_bytes(&bytes).expect("should deserialise");
    assert_eq!(restored.ip,          cfg.ip);
    assert_eq!(restored.prefix_len,  cfg.prefix_len);
    assert_eq!(restored.gateway,     cfg.gateway);
    assert_eq!(restored.broker_ip,   cfg.broker_ip);
    assert_eq!(restored.broker_port, cfg.broker_port);
    assert_eq!(restored.client_id,   cfg.client_id);
}

#[test]
fn custom_config_round_trip() {
    let mut cfg = NetworkConfig::default();
    cfg.ip          = [10, 0, 0, 5];
    cfg.gateway     = [10, 0, 0, 1];
    cfg.broker_ip   = [10, 0, 0, 2];
    cfg.broker_port = 8883;
    cfg.client_id   = heapless::String::try_from("factory-ctrl-01").unwrap();

    let bytes = cfg.to_bytes();
    let r = NetworkConfig::from_bytes(&bytes).unwrap();
    assert_eq!(r.ip,          [10,0,0,5]);
    assert_eq!(r.broker_port, 8883);
    assert_eq!(r.client_id.as_str(), "factory-ctrl-01");
}

#[test]
fn bad_magic_returns_none() {
    let mut bytes = NetworkConfig::default().to_bytes();
    bytes[0] = 0xFF; // corrupt magic
    assert!(NetworkConfig::from_bytes(&bytes).is_none());
}

#[test]
fn short_buffer_returns_none() {
    let bytes = [0u8; 10];
    assert!(NetworkConfig::from_bytes(&bytes).is_none());
}

// ---- credentials ----

#[test]
fn credentials_round_trip() {
    let mut cfg = NetworkConfig::default();
    cfg.mqtt_user = heapless::String::try_from("broker-user").unwrap();
    cfg.mqtt_pass = heapless::String::try_from("s3cr3t-pass").unwrap();
    cfg.web_user  = heapless::String::try_from("admin").unwrap();
    cfg.web_pass  = heapless::String::try_from("hunter2").unwrap();

    let r = NetworkConfig::from_bytes(&cfg.to_bytes()).unwrap();
    assert_eq!(r.mqtt_user.as_str(), "broker-user");
    assert_eq!(r.mqtt_pass.as_str(), "s3cr3t-pass");
    assert_eq!(r.web_user.as_str(),  "admin");
    assert_eq!(r.web_pass.as_str(),  "hunter2");
    // Original network fields survive alongside the new ones.
    assert_eq!(r.ip, cfg.ip);
    assert_eq!(r.client_id, cfg.client_id);
}

#[test]
fn max_length_credentials_round_trip() {
    let full = "A".repeat(jzf407_logic::config::CRED_MAX); // exactly 32 bytes, no NUL terminator
    let mut cfg = NetworkConfig::default();
    cfg.mqtt_pass = heapless::String::try_from(full.as_str()).unwrap();

    let r = NetworkConfig::from_bytes(&cfg.to_bytes()).unwrap();
    assert_eq!(r.mqtt_pass.as_str(), full.as_str());
}

#[test]
fn default_has_empty_credentials() {
    let cfg = NetworkConfig::default();
    assert!(cfg.mqtt_user.is_empty());
    assert!(cfg.web_pass.is_empty());
}

#[test]
fn legacy_image_upgrades_to_empty_credentials() {
    // Simulate a device flashed before credentials existed: the 49-byte legacy
    // image, padded out to the new length with 0xFF (a blank AT24C02). The new
    // parser must keep the network config and yield empty credentials, not bail.
    let cfg = NetworkConfig::default();
    let full = cfg.to_bytes(); // 175 bytes
    let mut legacy = full;
    for b in legacy.iter_mut().skip(49) {
        *b = 0xFF; // everything past the old layout is unwritten flash
    }
    let r = NetworkConfig::from_bytes(&legacy).expect("legacy image must still parse");
    assert_eq!(r.ip, cfg.ip);
    assert_eq!(r.client_id, cfg.client_id);
    assert!(r.mqtt_user.is_empty());
    assert!(r.mqtt_pass.is_empty());
    assert!(r.web_user.is_empty());
    assert!(r.web_pass.is_empty());
}


// ---- malformed EEPROM images / boot safety ----

#[test]
fn all_valid_prefixes_survive_round_trip() {
    for prefix in 1..=32 {
        let mut cfg = NetworkConfig::default();
        cfg.prefix_len = prefix;
        let restored = NetworkConfig::from_bytes(&cfg.to_bytes()).unwrap();
        assert_eq!(restored.prefix_len, prefix);
    }
}

#[test]
fn invalid_prefixes_from_eeprom_are_rejected() {
    for prefix in [0, 33, 64, 255] {
        let mut bytes = NetworkConfig::default().to_bytes();
        bytes[8] = prefix;
        assert!(
            NetworkConfig::from_bytes(&bytes).is_none(),
            "prefix_len={prefix} must not reach Ipv4Cidr::new at boot"
        );
    }
}

#[test]
fn zero_broker_port_from_eeprom_is_rejected() {
    let mut bytes = NetworkConfig::default().to_bytes();
    bytes[20] = 0;
    bytes[21] = 0;
    assert!(NetworkConfig::from_bytes(&bytes).is_none());
}

#[test]
fn invalid_utf8_client_id_rejects_whole_config() {
    let mut bytes = NetworkConfig::default().to_bytes();
    bytes[23] = 0xFF;
    bytes[24] = 0;
    assert!(NetworkConfig::from_bytes(&bytes).is_none());
}

#[test]
fn every_magic_byte_is_checked() {
    for index in 0..4 {
        let mut bytes = NetworkConfig::default().to_bytes();
        bytes[index] ^= 0x55;
        assert!(NetworkConfig::from_bytes(&bytes).is_none());
    }
}
