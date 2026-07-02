//! Network config persisted in AT24C02 at byte offset 16 (output state lives at 0).
//!
//! On-EEPROM layout, 175 bytes at 16..191, addresses absolute:
//! ```text
//!   [16..20)   magic 0xC0 0x4F 0x19 0x1E
//!   [20..24)   device IP;  [24] prefix_len;  [25..28) unused
//!   [28..32)   gateway
//!   [32..36)   broker IP;  [36..38) broker port (BE u16);  [38] reserved
//!   [39..63)   client_id, NUL-terminated, max 24
//!   [63..95)   MQTT user;   [95..127)  MQTT pass   (NUL-terminated, max 32)
//!   [127..159) web user;    [159..191) web pass    (NUL-terminated, max 32)
//! ```
//!
//! No CRC: parsing validates magic, prefix_len (1..=32 — anything else would
//! panic in Ipv4Cidr::new at boot and reset-loop the board) and non-zero port;
//! any failure falls back to Default. Credential slots parse leniently to empty
//! strings so a pre-credentials EEPROM image keeps its network config.

pub const DEFAULT_IP: [u8; 4] = [192, 168, 137, 2];
pub const DEFAULT_GW: [u8; 4] = [192, 168, 137, 1];
pub const DEFAULT_BROKER: [u8; 4] = [192, 168, 137, 1];
pub const DEFAULT_PORT: u16 = 1883;
pub const DEFAULT_PREFIX: u8 = 24;
pub const DEFAULT_ID: &str = "stm32-jzf407";

const MAGIC: [u8; 4] = [0xC0, 0x4F, 0x19, 0x1E];

pub const LEN: usize = 175;
pub const CRED_MAX: usize = 32;

#[derive(Clone, Debug)]
pub struct NetworkConfig {
    pub ip: [u8; 4],
    pub prefix_len: u8,
    pub gateway: [u8; 4],
    pub broker_ip: [u8; 4],
    pub broker_port: u16,
    pub client_id: heapless::String<24>,
    pub mqtt_user: heapless::String<CRED_MAX>,
    pub mqtt_pass: heapless::String<CRED_MAX>,
    pub web_user: heapless::String<CRED_MAX>,
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

fn read_str<const N: usize>(slot: &[u8]) -> heapless::String<N> {
    let end = slot.iter().position(|&c| c == 0).unwrap_or(slot.len());
    match core::str::from_utf8(&slot[..end]) {
        Ok(s) => heapless::String::try_from(s).unwrap_or_default(),
        Err(_) => heapless::String::new(),
    }
}

impl NetworkConfig {
    pub fn to_bytes(&self) -> [u8; LEN] {
        let mut b = [0u8; LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..8].copy_from_slice(&self.ip);
        b[8] = self.prefix_len;
        b[12..16].copy_from_slice(&self.gateway);
        b[16..20].copy_from_slice(&self.broker_ip);
        b[20] = (self.broker_port >> 8) as u8;
        b[21] = self.broker_port as u8;
        let id = self.client_id.as_bytes();
        let n = id.len().min(24);
        b[23..23 + n].copy_from_slice(&id[..n]);

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

        // prefix_len outside 1..=32 would panic in Ipv4Cidr::new at boot.
        let prefix_len = b[8];
        if !(1..=32).contains(&prefix_len) {
            return None;
        }

        let port = ((b[20] as u16) << 8) | (b[21] as u16);
        if port == 0 {
            return None;
        }

        let id_bytes = &b[23..47];
        let end = id_bytes.iter().position(|&c| c == 0).unwrap_or(24);
        let id = core::str::from_utf8(&id_bytes[..end]).ok()?;

        Some(Self {
            ip: [b[4], b[5], b[6], b[7]],
            prefix_len,
            gateway: [b[12], b[13], b[14], b[15]],
            broker_ip: [b[16], b[17], b[18], b[19]],
            broker_port: port,
            client_id: heapless::String::try_from(id).ok()?,
            mqtt_user: read_str(&b[47..79]),
            mqtt_pass: read_str(&b[79..111]),
            web_user: read_str(&b[111..143]),
            web_pass: read_str(&b[143..175]),
        })
    }
}

fn write_str(slot: &mut [u8], s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(slot.len());
    slot[..n].copy_from_slice(&bytes[..n]);
}

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

pub fn parse_port(s: &str) -> Option<u16> {
    let p: u16 = s.trim().parse().ok()?;
    if p == 0 {
        return None;
    }
    Some(p)
}
