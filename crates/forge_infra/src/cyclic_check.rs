//! CRC-8 / CRC-16 / CRC-32 checksums (table-free bit-serial implementations).
//!
//! - [`crc8`] — polynomial 0x07 (ATM HEC, SMBus, many 1-Wire devices)
//! - [`crc16_ccitt`] — polynomial 0x1021, init 0xFFFF (XMODEM, Bluetooth HCI)
//! - [`crc32_ieee`] — polynomial 0xEDB88320 (Ethernet, gzip, PNG, zip)
//!
//! All implementations are table-free — slow but correct without a 256-entry
//! lookup table. For high-throughput use, swap to a table-driven version.

/// CRC-8 with polynomial 0x07 (LSB-first, no reflection).
pub fn crc8(data: &[u8], init: u8) -> u8 {
    let mut crc = init;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// CRC-16-CCITT (polynomial 0x1021, init 0xFFFF, no reflection).
pub fn crc16_ccitt(data: &[u8], init: u16) -> u16 {
    let mut crc = init;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// CRC-32-IEEE (polynomial 0xEDB88320, init 0xFFFFFFFF, finalize XOR 0xFFFFFFFF).
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_known_vector() {
        assert_eq!(crc8(b"123456789", 0), 0xF4);
    }

    #[test]
    fn crc16_xmodem_vector() {
        // XMODEM uses init 0x0000: "123456789" -> 0x31C3
        assert_eq!(crc16_ccitt(b"123456789", 0x0000), 0x31C3);
    }

    #[test]
    fn crc32_ieee_known_vector() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn crc32_ieee_empty_input() {
        assert_eq!(crc32_ieee(b""), 0);
    }
}