//! Per-family GATT map: which characteristic does what, and the framing constants that go with it.
//!
//! Roles for WHOOP 4 come from NOOP's `PROTOCOL_WHOOP4.md` (command write / command-response notify /
//! event notify / data notify). WHOOP 5 uses the same numbering (`fd4b0002`..`0005`) plus an extra
//! `fd4b0007` notify channel; NOOP documents the UUIDs but not the per-characteristic routing, so
//! the WHOOP 5 role assignment is by analogy and flagged `verified: false`. In practice a client
//! should subscribe to *all* notify characteristics and route by packet type, which is what NOOP does.

use super::DeviceFamily;

/// What a custom-service characteristic is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharacteristicRole {
    /// Host to strap: command frames (write with response). `…0002`.
    CommandWrite,
    /// Strap to host notifications carrying command responses. `…0003`.
    CommandResponse,
    /// Strap to host notifications carrying events. `…0004`.
    Events,
    /// Strap to host notifications carrying (fragmented) data: realtime, history, metadata. `…0005`.
    Data,
    /// WHOOP 5 only (`fd4b0007`): purpose undocumented. Subscribe and feed to a reassembler anyway.
    Auxiliary,
}

/// One characteristic of a family's custom service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Characteristic {
    /// Role of this characteristic.
    pub role: CharacteristicRole,
    /// Lowercase hyphenated 128-bit UUID.
    pub uuid: &'static str,
    /// The host writes frames here.
    pub write: bool,
    /// The strap sends notifications here (subscribe to it).
    pub notify: bool,
    /// `true` when the role (not only the UUID) is documented for this family.
    pub role_verified: bool,
}

const fn ch(
    role: CharacteristicRole,
    uuid: &'static str,
    write: bool,
    notify: bool,
    role_verified: bool,
) -> Characteristic {
    Characteristic { role, uuid, write, notify, role_verified }
}

const W4_CHARS: [Characteristic; 4] = [
    ch(CharacteristicRole::CommandWrite, "61080002-8d6d-82b8-614a-1c8cb0f8dcc6", true, false, true),
    ch(CharacteristicRole::CommandResponse, "61080003-8d6d-82b8-614a-1c8cb0f8dcc6", false, true, true),
    ch(CharacteristicRole::Events, "61080004-8d6d-82b8-614a-1c8cb0f8dcc6", false, true, true),
    ch(CharacteristicRole::Data, "61080005-8d6d-82b8-614a-1c8cb0f8dcc6", false, true, true),
];

const W5_CHARS: [Characteristic; 5] = [
    ch(CharacteristicRole::CommandWrite, "fd4b0002-cce1-4033-93ce-002d5875f58a", true, false, true),
    ch(CharacteristicRole::CommandResponse, "fd4b0003-cce1-4033-93ce-002d5875f58a", false, true, false),
    ch(CharacteristicRole::Events, "fd4b0004-cce1-4033-93ce-002d5875f58a", false, true, false),
    ch(CharacteristicRole::Data, "fd4b0005-cce1-4033-93ce-002d5875f58a", false, true, false),
    ch(CharacteristicRole::Auxiliary, "fd4b0007-cce1-4033-93ce-002d5875f58a", false, true, false),
];

/// Standard Bluetooth SIG Heart Rate service (`180D`), works unbonded on both generations.
pub const STANDARD_HEART_RATE_SERVICE: &str = "0000180d-0000-1000-8000-00805f9b34fb";
/// Heart Rate Measurement characteristic (`2A37`), notify. See [`super::std_hr`].
pub const STANDARD_HEART_RATE_MEASUREMENT: &str = "00002a37-0000-1000-8000-00805f9b34fb";
/// Standard Battery service (`180F`). Some WHOOP 4 straps report a constant 100; prefer command 26.
pub const STANDARD_BATTERY_SERVICE: &str = "0000180f-0000-1000-8000-00805f9b34fb";
/// Battery Level characteristic (`2A19`), single byte percent.
pub const STANDARD_BATTERY_LEVEL: &str = "00002a19-0000-1000-8000-00805f9b34fb";

/// Which checksum guards a frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderCrcKind {
    /// CRC-8 (poly `0x07`) over the two length bytes. WHOOP 4.
    Crc8,
    /// CRC-16/MODBUS over the first six header bytes. WHOOP 5.
    Crc16Modbus,
}

/// The fixed 16-byte WHOOP 5 `CLIENT_HELLO` frame (a type-35 `GET_HELLO`, command 145, revision 1,
/// sequence 1) that must be written to the command characteristic right after discovery.
pub const WHOOP5_CLIENT_HELLO: [u8; 16] = [
    0xAA, 0x01, 0x08, 0x00, 0x00, 0x01, 0xE6, 0x71, 0x23, 0x01, 0x91, 0x01, 0x36, 0x3E, 0x5C, 0x8D,
];

impl DeviceFamily {
    /// All custom-service characteristics of this family, in ascending UUID order.
    pub fn characteristics(self) -> &'static [Characteristic] {
        match self {
            DeviceFamily::Whoop4 => &W4_CHARS,
            DeviceFamily::Whoop5 => &W5_CHARS,
        }
    }

    /// The characteristic with the given role, if this family has one.
    pub fn characteristic(self, role: CharacteristicRole) -> Option<&'static Characteristic> {
        self.characteristics().iter().find(|c| c.role == role)
    }

    /// UUID of the command-write characteristic (`…0002`).
    pub fn command_characteristic_uuid(self) -> &'static str {
        self.characteristics()[0].uuid
    }

    /// Characteristics to subscribe to (every `notify` one).
    pub fn notify_characteristics(self) -> impl Iterator<Item = &'static Characteristic> {
        self.characteristics().iter().filter(|c| c.notify)
    }

    /// Role of a characteristic UUID (case-insensitive), if it belongs to this family's service.
    pub fn role_of_uuid(self, uuid: &str) -> Option<CharacteristicRole> {
        let u = uuid.to_ascii_lowercase();
        self.characteristics().iter().find(|c| c.uuid == u).map(|c| c.role)
    }

    /// Which checksum guards this family's frame header.
    pub fn header_crc_kind(self) -> HeaderCrcKind {
        match self {
            DeviceFamily::Whoop4 => HeaderCrcKind::Crc8,
            DeviceFamily::Whoop5 => HeaderCrcKind::Crc16Modbus,
        }
    }

    /// Bytes of fixed header preceding the inner record: 4 on WHOOP 4, 8 on WHOOP 5.
    pub fn header_len(self) -> usize {
        match self {
            DeviceFamily::Whoop4 => 4,
            DeviceFamily::Whoop5 => 8,
        }
    }

    /// Smallest frame accepted: 11 bytes on WHOOP 4 (zero-payload frame), 13 on WHOOP 5
    /// (NOOP's empirical floor: header 8 + at least the type byte + CRC32).
    pub fn min_frame_len(self) -> usize {
        match self {
            DeviceFamily::Whoop4 => 11,
            DeviceFamily::Whoop5 => 13,
        }
    }

    /// Fixed frame written to the command characteristic right after discovery, if the family
    /// needs one. `Some` for WHOOP 5, `None` for WHOOP 4 (whose first write is a confirmed
    /// `GET_BATTERY_LEVEL`).
    pub fn client_hello(self) -> Option<&'static [u8]> {
        match self {
            DeviceFamily::Whoop4 => None,
            DeviceFamily::Whoop5 => Some(&WHOOP5_CLIENT_HELLO),
        }
    }

    /// Conservative ATT MTU to assume when none is negotiated (the BLE minimum, 23: 20 payload
    /// bytes per notification). **Unverified for real straps**: NOOP states only that notifications
    /// on `…0005` split frames, not the negotiated MTU. The [`Reassembler`](super::Reassembler)
    /// does not depend on it; it matters only to the emulator's [`fragment`](super::fragment).
    pub fn assumed_att_mtu(self) -> usize {
        23
    }
}
