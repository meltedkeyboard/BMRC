//! `.bmrc` file format.
//!
//! BMRC stream contains a 14-byte header followed by the payload
//!
//! Offset Size Field
//! 0      4    Magic bytes: "BMR1"
//! 4      1    Compression level (1-10)
//! 5      1    Flags (bit 0 = FLAG_STORED: payload is not compressed)
//! 6      8    Original data size as a little-endian u64
//! 14     ..   Payload. Raw data or compressed data
//! `
//!
//! The header is always 14 bytes long and contains the information
//! needed to decompress the stream

use crate::error::BmrcError;

pub const MAGIC: [u8; 4] = *b"BMR1";

/// Flag bit: payload is stored as-is (no entropy coding)
pub const FLAG_STORED: u8 = 0x01;

/// Flag bit: BWT pre-pass was applied before entropy coding.
///
/// When set, the first 4 bytes of the payload hold the BWT primary index
/// (little-endian u32), followed by the entropy-coded BWT output.
pub const FLAG_BWT: u8 = 0x02;

/// Total size of the container header in bytes
pub const HEADER_LEN: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Ignored when [`FLAG_STORED`] is set, but still recorded for diagnostics
    pub level: u8,
    /// Flag bits, see [`FLAG_STORED`]
    pub flags: u8,
    /// Length of the original, uncompressed data in bytes
    pub original_len: u64,
}

impl Header {
    /// Serialize the header and append it to `out`
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        out.push(self.level);
        out.push(self.flags);
        out.extend_from_slice(&self.original_len.to_le_bytes());
    }

    pub fn read(data: &[u8]) -> Result<(Header, &[u8]), BmrcError> {
        if data.len() < HEADER_LEN {
            return Err(BmrcError::HeaderTooShort);
        }
        if data[0..4] != MAGIC {
            return Err(BmrcError::BadMagic);
        }
        let level = data[4];
        let flags = data[5];
        let original_len = u64::from_le_bytes(data[6..14].try_into().unwrap());
        Ok((
            Header {
                level,
                flags,
                original_len,
            },
            &data[HEADER_LEN..],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_header() {
        let h = Header {
            level: 7,
            flags: FLAG_STORED,
            original_len: 123_456_789,
        };
        let mut buf = Vec::new();
        h.write(&mut buf);
        buf.extend_from_slice(b"payload-bytes");

        let (parsed, rest) = Header::read(&buf).unwrap();
        assert_eq!(parsed, h);
        assert_eq!(rest, b"payload-bytes");
    }
}
