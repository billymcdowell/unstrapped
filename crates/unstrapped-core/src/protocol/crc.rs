//! Checksums used by the WHOOP wire format. All three are table driven and hand rolled.
//!
//! | Name        | Width | Poly                | Init         | Reflect in/out | XorOut       | Used for                     |
//! |-------------|-------|---------------------|--------------|----------------|--------------|------------------------------|
//! | CRC-8       | 8     | `0x07`              | `0x00`       | no / no        | `0x00`       | WHOOP 4 header (length bytes) |
//! | CRC-16/MODBUS | 16  | `0x8005` (`0xA001` reflected) | `0xFFFF` | yes / yes | `0x0000`   | WHOOP 5 header (bytes 0..6)   |
//! | CRC-32 (zlib) | 32  | `0x04C11DB7` (`0xEDB88320` reflected) | `0xFFFFFFFF` | yes / yes | `0xFFFFFFFF` | frame body, both families |

const fn build_crc8_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u8;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const fn build_crc16_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u16;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xA001 } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const fn build_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC8_TABLE: [u8; 256] = build_crc8_table();
static CRC16_TABLE: [u16; 256] = build_crc16_table();
static CRC32_TABLE: [u32; 256] = build_crc32_table();

/// CRC-8, polynomial `0x07`, init `0x00`, no reflection, no final XOR.
///
/// WHOOP 4 computes it over the two little-endian length bytes only.
pub fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |crc, &b| CRC8_TABLE[(crc ^ b) as usize])
}

/// CRC-16/MODBUS: reflected, polynomial `0xA001`, init `0xFFFF`, no final XOR.
///
/// WHOOP 5 computes it over frame bytes `0..6` and stores it little-endian at bytes `6..8`.
pub fn crc16_modbus(data: &[u8]) -> u16 {
    data.iter()
        .fold(0xFFFFu16, |crc, &b| (crc >> 8) ^ CRC16_TABLE[((crc ^ b as u16) & 0xFF) as usize])
}

/// Standard zlib / IEEE CRC-32: reflected, polynomial `0xEDB88320`, init and final XOR `0xFFFFFFFF`.
///
/// Both families compute it over the inner record (`type, seq, cmd, payload` and, on WHOOP 5, the
/// zero padding) and store it little-endian as the last four bytes of the frame.
pub fn crc32(data: &[u8]) -> u32 {
    !data
        .iter()
        .fold(0xFFFF_FFFFu32, |crc, &b| (crc >> 8) ^ CRC32_TABLE[((crc ^ b as u32) & 0xFF) as usize])
}
