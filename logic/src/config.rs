//! Network configuration persisted in the AT24C02 EEPROM at byte offset 16
//! (runtime output state lives separately at offset 0 — see `persistence`).
//!
//! On-EEPROM layout: 175 bytes spanning [16..191), addresses absolute:
//!   [16..20)   magic 0xC0 0x4F 0x19 0x1E  — rejects blank / foreign EEPROMs
//!   [20..24)   device IP (4 octets)
//!   [24]       prefix_len (CIDR bits, e.g. 24);  [25..28) unused
//!   [28..32)   gateway (4 octets)
//!   [32..36)   broker IP (4 octets)
//!   [36..38)   broker port (big-endian u16)
//!   [38]       reserved (was a DHCP flag; firmware is static-IP only)
//!   [39..63)   client_id, NUL-terminated, max 24 bytes
//!   [63..95)   MQTT username, NUL-terminated, max 32 bytes
//!   [95..127)  MQTT password, NUL-terminated, max 32 bytes
//!   [127..159) web (HTTP Basic Auth) username, NUL-terminated, max 32 bytes
//!   [159..191) web (HTTP Basic Auth) password, NUL-terminated, max 32 bytes
//!
//! The four credential fields were appended after the original 49-byte layout.
//! A device flashed before this change has 0xFF (blank) in [63..191); those
//! bytes parse leniently to empty strings (see `read_str`), so an upgrade keeps
//! the existing network config and simply boots with no credentials configured
//! (anonymous MQTT + open web page) — exactly the pre-auth behaviour.
//!
//! Validation is magic-only: a wrong magic falls back to Default. There is no
//! CRC — a torn EEPROM write can yield a struct that passes the magic check, so
//! callers treat a bad-looking config by reverting to defaults, not by trusting it.
//!
//! These are plain-data structs with no hardware deps, so (de)serialisation is
//! unit-tested natively on the host — see tests/config_parser.rs.

pub const DEFAULT_IP: [u8; 4] = [192, 168, 137, 2];
pub const DEFAULT_GW: [u8; 4] = [192, 168, 137, 1];
pub const DEFAULT_BROKER: [u8; 4] = [192, 168, 137, 1];
pub const DEFAULT_PORT: u16 = 1883;
pub const DEFAULT_PREFIX: u8 = 24;
pub const DEFAULT_ID: &str = "stm32-jzf407";

const MAGIC: [u8; 4] = [0xC0, 0x4F, 0x19, 0x1E];

/// Total size of the serialised config image in EEPROM (the firmware reads/writes
/// exactly this many bytes starting at offset 16). Public so the firmware side
/// can size its buffer from one source of truth.
pub const LEN: usize = 175;

/// Max length (in bytes) of each credential field. Matches the heapless::String
/// capacity below and the on-EEPROM slot width.
pub const CRED_MAX: usize = 32;

#[derive(Clone, Debug)]
pub struct NetworkConfig {
    pub ip: [u8; 4],
    pub prefix_len: u8,
    pub gateway: [u8; 4],
    pub broker_ip: [u8; 4],
    pub broker_port: u16,
    pub client_id: heapless::String<24>,
    /// MQTT CONNECT username. Empty = connect anonymously (no auth sent).
    pub mqtt_user: heapless::String<CRED_MAX>,
    /// MQTT CONNECT password. Empty = no password sent.
    pub mqtt_pass: heapless::String<CRED_MAX>,
    /// HTTP Basic Auth username. Empty user AND pass = web page is open (no auth).
    pub web_user: heapless::String<CRED_MAX>,
    /// HTTP Basic Auth password.
    pub web_pass: heapless::String<CRED_MAX>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            ip: DEFAULT_IP,
            prefix_len: DEFAULT_PREFIX,
            gateway: DEFAULT_GW,
            broker_ip: DEFAULT_BROKER,
            broker_port: DEFAULT_PORT,
            client_id: heapless::String::try_from(DEFAULT_ID).unwrap_or_default(),
            mqtt_user: heapless::String::new(),
            mqtt_pass: heapless::String::new(),
            web_user: heapless::String::new(),
            web_pass: heapless::String::new(),
        }
    }
}

/// Read a NUL-terminated string field from an EEPROM slice, leniently.
///
/// Used for the credential fields, which may be 0xFF (blank) on a device flashed
/// before they existed. Anything that isn't valid UTF-8 up to the first NUL (or
/// the end of the slot) yields an empty string rather than failing the whole
/// config parse — so an upgrade never wipes the network config, it just boots
/// with no credentials.
fn read_str<const N: usize>(slot: &[u8]) -> heapless::String<N> {
    let end = slot.iter().position(|&c| c == 0).unwrap_or(slot.len());
    match core::str::from_utf8(&slot[..end]) {
        Ok(s) => heapless::String::try_from(s).unwrap_or_default(),
        Err(_) => heapless::String::new(),
    }
}

impl NetworkConfig {
    /// Serialise into the fixed 49-byte EEPROM image. Offsets here are relative
    /// to the buffer start (EEPROM byte 16); see the module-level layout doc.
    pub fn to_bytes(&self) -> [u8; LEN] {
        let mut b = [0u8; LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..8].copy_from_slice(&self.ip);
        b[8] = self.prefix_len;
        b[12..16].copy_from_slice(&self.gateway);
        b[16..20].copy_from_slice(&self.broker_ip);
        b[20] = (self.broker_port >> 8) as u8;
        b[21] = self.broker_port as u8;
        // b[22] is reserved (former DHCP flag) — left zero to keep the layout stable.
        let id = self.client_id.as_bytes();
        let n = id.len().min(24);
        b[23..23 + n].copy_from_slice(&id[..n]);

        // Credential slots: 32 bytes each, NUL-padded (the buffer starts zeroed).
        write_str(&mut b[47..79], &self.mqtt_user);
        write_str(&mut b[79..111], &self.mqtt_pass);
        write_str(&mut b[111..143], &self.web_user);
        write_str(&mut b[143..175], &self.web_pass);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < LEN {
            return None;
        }
        if b[0..4] != MAGIC {
            return None;
        }

        let port = ((b[20] as u16) << 8) | (b[21] as u16);
        let id_bytes = &b[23..47];
        let end = id_bytes.iter().position(|&c| c == 0).unwrap_or(24);
        let id = core::str::from_utf8(&id_bytes[..end]).ok()?;

        Some(Self {
            ip: [b[4], b[5], b[6], b[7]],
            prefix_len: b[8],
            gateway: [b[12], b[13], b[14], b[15]],
            broker_ip: [b[16], b[17], b[18], b[19]],
            broker_port: port,
            client_id: heapless::String::try_from(id).ok()?,
            // Lenient: blank/garbage credential slots → empty (see read_str).
            mqtt_user: read_str(&b[47..79]),
            mqtt_pass: read_str(&b[79..111]),
            web_user: read_str(&b[111..143]),
            web_pass: read_str(&b[143..175]),
        })
    }
}

/// Copy a string into a fixed EEPROM slot, truncated to the slot width. The
/// caller's buffer is pre-zeroed, so a string shorter than the slot is left
/// NUL-terminated automatically.
fn write_str(slot: &mut [u8], s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(slot.len());
    slot[..n].copy_from_slice(&bytes[..n]);
}

/// Validate a dot-decimal IPv4 string, return `[u8;4]` or `None`.
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = s.splitn(4, '.');
    for o in &mut octets {
        *o = parts.next()?.parse::<u8>().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(octets)
}

/// Validate port number string.
pub fn parse_port(s: &str) -> Option<u16> {
    let p: u16 = s.trim().parse().ok()?;
    if p == 0 {
        return None;
    }
    Some(p)
}
