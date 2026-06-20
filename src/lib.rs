//! BMRC is a lossless compression algorithm for general use.
//!
//! It compresses data with several statistical models and a
//! long-range match model. These models work together through
//! an adaptive mixer. The result is encoded with a binary range
//! coder.
//!
//! This crate can compress and decompress data stored in memory.
//! The `compress` function takes input data and returns compressed
//! bytes. The `decompress` function restores the original data.
//! The crate uses its own `.bmrc` container format.
//!
//! BMRC supports ten compression levels from 1 to 10.
//! Lower levels are faster and use fewer models.
//! Higher levels use more models and usually produce smaller files,
//! but compression is much slower.
//!
//! Example:
//!
//! ```
//! use bmrc::{compress, decompress};
//!
//! let data = b"hello hello hello hello world world world".repeat(10);
//! let compressed = compress(&data, 6);
//! let restored = decompress(&compressed).unwrap();
//! assert_eq!(restored, data);
//! assert!(compressed.len() < data.len());
//! ```
//!
//! A BWT pre-pass is available via [`compress_bwt`]. It reorders bytes to
//! cluster equal values before entropy coding, which can improve ratio on
//! text and structured binary data. The standard [`decompress`] detects and
//! reverses the BWT automatically.

pub mod bwt;
pub mod error;
pub mod format;
pub mod levels;
pub mod parallel;
pub mod predictor;
pub mod range_coder;

pub use error::BmrcError;
pub use levels::{config_for_level, estimated_memory_bytes, LevelConfig};
pub use parallel::{compress_parallel, decompress_parallel, DEFAULT_BLOCK_SIZE, PARALLEL_MAGIC};

use predictor::Predictor;
use range_coder::{Decoder, Encoder};

pub use format::{Header, FLAG_BWT, FLAG_STORED, HEADER_LEN, MAGIC};


pub const MIN_LEVEL: u8 = 1; /// fastest, weakest model
pub const MAX_LEVEL: u8 = 10; /// slowest, strongest model

/// Compresses `data` and returns a `.bmrc` byte stream.
///
/// The returned data can be restored with [`decompress`].
/// If compression does not make the data smaller, the input
/// is stored as-is with a small header. Because of this,
/// `compress` adds at most [`format::HEADER_LEN`] bytes.
pub fn compress(data: &[u8], level: u8) -> Vec<u8> {
    let level = level.clamp(MIN_LEVEL, MAX_LEVEL);

    let payload = encode_payload(data, level);

    if payload.len() >= data.len() {
        let mut out = Vec::with_capacity(HEADER_LEN + data.len());
        Header {
            level,
            flags: FLAG_STORED,
            original_len: data.len() as u64,
        }
            .write(&mut out);
        out.extend_from_slice(data);
        out
    } else {
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
        Header {
            level,
            flags: 0,
            original_len: data.len() as u64,
        }
            .write(&mut out);
        out.extend_from_slice(&payload);
        out
    }
}

/// Compresses `data` with a BWT pre-pass and returns a `.bmrc` byte stream.
///
/// The Burrows-Wheeler Transform reorders bytes so that equal bytes cluster
/// together, which improves prediction accuracy for the context models that
/// follow. On text and structured binary data this typically yields a better
/// ratio than [`compress`] alone.
///
/// The BWT primary index (4 bytes) is prepended to the payload and the
/// [`FLAG_BWT`] bit is set in the header. [`decompress`] detects the flag
/// and inverts the BWT automatically; no separate decompress function is
/// needed.
///
/// Falls back to plain [`compress`] when the BWT pre-pass does not reduce
/// the output size.
pub fn compress_bwt(data: &[u8], level: u8) -> Vec<u8> {
    let level = level.clamp(MIN_LEVEL, MAX_LEVEL);

    if data.is_empty() {
        return compress(data, level);
    }

    let (bwt_data, primary_idx) = bwt::bwt_encode(data);
    let payload = encode_payload(&bwt_data, level);

    // 4 extra bytes for the primary index stored in the payload.
    if payload.len() + 4 >= data.len() {
        return compress(data, level);
    }

    let mut out = Vec::with_capacity(HEADER_LEN + 4 + payload.len());
    Header {
        level,
        flags: FLAG_BWT,
        original_len: data.len() as u64,
    }
        .write(&mut out);
    out.extend_from_slice(&primary_idx.to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Decompresses a `.bmrc` byte stream created by [`compress`] or [`compress_bwt`].
///
/// # Errors
///
/// Returns [`BmrcError`] if the input is invalid.
/// This can happen if the data is too short, has an incorrect
/// BMRC header, or contains incomplete stored data.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, BmrcError> {
    let (header, payload) = Header::read(data)?;

    if header.flags & FLAG_STORED != 0 {
        let n = header.original_len as usize;
        if payload.len() < n {
            return Err(BmrcError::Truncated);
        }
        return Ok(payload[..n].to_vec());
    }

    if header.flags & FLAG_BWT != 0 {
        // First 4 payload bytes are the BWT primary index.
        if payload.len() < 4 {
            return Err(BmrcError::Truncated);
        }
        let primary_idx = u32::from_le_bytes(payload[..4].try_into().unwrap());
        let bwt_data = decode_payload(&payload[4..], header.level, header.original_len as usize);
        return Ok(bwt::bwt_decode(&bwt_data, primary_idx));
    }

    Ok(decode_payload(payload, header.level, header.original_len as usize))
}

/// Encodes `data` with the predictor and range coder.
///
/// Returns the encoded bitstream without the container header.
fn encode_payload(data: &[u8], level: u8) -> Vec<u8> {
    let mut predictor = Predictor::new(level);
    let mut enc = Encoder::new();

    for &byte in data {
        predictor.start_byte();
        for i in (0..8).rev() {
            let bit = (byte >> i) & 1;
            let p = predictor.predict();
            enc.encode_bit(bit, p);
            predictor.update(bit);
        }
        predictor.end_byte(byte);
    }

    enc.finish()
}

/// Inverse of [`encode_payload`]: decode `orig_len` bytes from `payload`.
fn decode_payload(payload: &[u8], level: u8, orig_len: usize) -> Vec<u8> {
    let mut predictor = Predictor::new(level);
    let mut dec = Decoder::new(payload);
    let mut out = Vec::with_capacity(orig_len);

    for _ in 0..orig_len {
        predictor.start_byte();
        let mut byte = 0u8;
        for _ in 0..8 {
            let p = predictor.predict();
            let bit = dec.decode_bit(p);
            predictor.update(bit);
            byte = (byte << 1) | bit;
        }
        predictor.end_byte(byte);
        out.push(byte);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input() {
        let c = compress(&[], 5);
        let d = decompress(&c).unwrap();
        assert!(d.is_empty());
    }

    #[test]
    fn small_input_all_levels() {
        let data = b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        for level in 1..=10u8 {
            let c = compress(data, level);
            let d = decompress(&c).unwrap();
            assert_eq!(d, data, "level {level}");
        }
    }

    #[test]
    fn level_is_clamped() {
        let c0 = compress(b"hello world", 0);
        let c11 = compress(b"hello world", 11);
        let c1 = compress(b"hello world", 1);
        let c10 = compress(b"hello world", 10);
        assert_eq!(c0[4], 1);
        assert_eq!(c11[4], 10);
        assert_eq!(c0, c1);
        assert_eq!(c11, c10);
    }

    #[test]
    fn incompressible_random_data_is_stored() {
        // PRNG-generated data should not expand by more than the header.
        let mut data = vec![0u8; 4096];
        let mut x: u32 = 0xDEADBEEF;
        for b in data.iter_mut() {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (x >> 24) as u8;
        }
        let c = compress(&data, 10);
        let d = decompress(&c).unwrap();
        assert_eq!(d, data);
        assert!(c.len() <= data.len() + HEADER_LEN);
    }

    #[test]
    fn bad_magic_errors() {
        let err = decompress(b"not a bmrc stream at all!!").unwrap_err();
        assert_eq!(err, BmrcError::BadMagic);
    }

    #[test]
    fn header_too_short_errors() {
        let err = decompress(b"NX").unwrap_err();
        assert_eq!(err, BmrcError::HeaderTooShort);
    }

    #[test]
    fn bwt_compress_roundtrip_text() {
        let data = b"the quick brown fox jumps over the lazy dog. ".repeat(50);
        for level in [1u8, 5, 10] {
            let c = compress_bwt(&data, level);
            let d = decompress(&c).unwrap();
            assert_eq!(d, data.as_slice(), "BWT roundtrip failed at level {level}");
        }
    }

    #[test]
    fn bwt_compress_falls_back_for_incompressible_data() {
        // Random-ish data: BWT pre-pass should not help, expect plain compress output.
        let mut data = vec![0u8; 200];
        let mut x: u32 = 0xCAFEBABE;
        for b in data.iter_mut() {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (x >> 24) as u8;
        }
        let c = compress_bwt(&data, 5);
        let d = decompress(&c).unwrap();
        assert_eq!(d, data);
    }
}