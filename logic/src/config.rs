//! Network configuration stored in AT24C02 at offset 16.
//!
//! Layout (bytes 16..79):
//!   [16..19]  magic  0xC0, 0x4F, 0x19, 0x1E
//!   [20..23]  device_ip
//!   [24..27]  netmask (stored as prefix_len u8 at [24], others unused)
//!   [28..31]  gateway
//!   [32..35]  broker_ip
//!   [36..37]  broker_port (big-endian u16)
//!   [38]      dhcp (0/1)
//!   [39..63]  client_id (null-terminated, max 24 chars)
//!   [64]      CRC8 of bytes [16..64]
//!
//! Pure structs are no_std + unit-testable natively.

pub const DEFAULT_IP: [u8; 4] = [192, 168, 137, 2];
pub const DEFAULT_GW: [u8; 4] = [192, 168, 137, 1];
pub const DEFAULT_BROKER: [u8; 4] = [192, 168, 137, 1];
pub const DEFAULT_PORT: u16 = 1883;
pub const DEFAULT_DHCP: bool = false;
pub const DEFAULT_PREFIX: u8 = 24;
pub const DEFAULT_ID: &str = "stm32-jzf407";

const MAGIC: [u8; 4] = [0xC0, 0x4F, 0x19, 0x1E];
const LEN: usize = 49; // bytes [16..65) used for CRC

#[derive(Clone, Debug)]
pub struct NetworkConfig {
    pub ip: [u8; 4],
    pub prefix_len: u8,
    pub gateway: [u8; 4],
    pub broker_ip: [u8; 4],
    pub broker_port: u16,
    pub dhcp: bool,
    pub client_id: heapless::String<24>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            ip: DEFAULT_IP,
            prefix_len: DEFAULT_PREFIX,
            gateway: DEFAULT_GW,
            broker_ip: DEFAULT_BROKER,
            broker_port: DEFAULT_PORT,
            dhcp: DEFAULT_DHCP,
            client_id: heapless::String::try_from(DEFAULT_ID).unwrap_or_default(),
        }
    }
}

impl NetworkConfig {
    /// Serialise to 49-byte buffer (no CRC byte — caller appends separately).
    pub fn to_bytes(&self) -> [u8; LEN] {
        let mut b = [0u8; LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..8].copy_from_slice(&self.ip);
        b[8] = self.prefix_len;
        b[12..16].copy_from_slice(&self.gateway);
        b[16..20].copy_from_slice(&self.broker_ip);
        b[20] = (self.broker_port >> 8) as u8;
        b[21] = self.broker_port as u8;
        b[22] = self.dhcp as u8;
        let id = self.client_id.as_bytes();
        let n = id.len().min(24);
        b[23..23 + n].copy_from_slice(&id[..n]);
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
            dhcp: b[22] != 0,
            client_id: heapless::String::try_from(id).ok()?,
        })
    }
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
