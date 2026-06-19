//! Block-parallel compression and decompression.
//!
//! [`compress_parallel`] splits the input into fixed-size blocks, compresses
//! each block independently on a separate OS thread, then wraps the results
//! in a `.bmrp` envelope. [`decompress_parallel`] reverses the process.
//!
//! Because each block is an independent BMRC stream the predictor state does
//! not cross block boundaries. Larger blocks compress better but reduce
//! parallelism; smaller blocks increase parallelism at the cost of ratio.
//! 256 KB (`DEFAULT_BLOCK_SIZE`) is a reasonable default for most workloads.
//!
//! ## Container format (`.bmrp`)
//!
//! ```text
//! Offset    Size      Field
//! 0         4         Magic "BMRP"
//! 4         4         Block count N (u32 LE)
//! 8         4 * N     Block payload sizes in bytes (u32 LE each)
//! 8 + 4*N   ...       Block payloads -- each is a complete BMRC (.bmrc) stream
//! ```

use std::thread;

use crate::{compress, decompress, BmrcError};

/// Magic bytes for the parallel container format.
pub const PARALLEL_MAGIC: [u8; 4] = *b"BMRP";

/// Default block size for [`compress_parallel`] when `block_size` is `None`.
pub const DEFAULT_BLOCK_SIZE: usize = 256 * 1024;

/// Compresses `data` in parallel using block-level threading.
///
/// The input is split into blocks of `block_size` bytes (or
/// [`DEFAULT_BLOCK_SIZE`] when `None`). Each block is compressed
/// independently at the given `level` using a fresh predictor. All blocks
/// are dispatched concurrently on OS threads and results are reassembled in
/// the original order.
///
/// The returned bytes form a `.bmrp` stream that [`decompress_parallel`] can
/// restore. The stream is self-describing: `level` and block count are stored
/// inside the individual `.bmrc` sub-streams and the outer header respectively.
pub fn compress_parallel(data: &[u8], level: u8, block_size: Option<usize>) -> Vec<u8> {
    let block_size = block_size.unwrap_or(DEFAULT_BLOCK_SIZE).max(1);

    // Always produce at least one block so the empty-input case round-trips.
    let blocks: Vec<&[u8]> = if data.is_empty() {
        vec![data]
    } else {
        data.chunks(block_size).collect()
    };

    let n = blocks.len();

    let compressed_blocks: Vec<Vec<u8>> = thread::scope(|s| {
        let handles: Vec<_> = blocks
            .iter()
            .map(|block| s.spawn(move || compress(block, level)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("compressor thread panicked"))
            .collect()
    });

    let payload_total: usize = compressed_blocks.iter().map(|b| b.len()).sum();
    let mut out = Vec::with_capacity(4 + 4 + 4 * n + payload_total);

    out.extend_from_slice(&PARALLEL_MAGIC);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    for b in &compressed_blocks {
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    }
    for b in compressed_blocks {
        out.extend_from_slice(&b);
    }

    out
}

/// Decompresses a `.bmrp` stream produced by [`compress_parallel`].
///
/// All blocks are dispatched concurrently on OS threads and reassembled in
/// the original order.
///
/// # Errors
///
/// Returns [`BmrcError`] if the stream header is malformed or any block
/// fails to decompress.
pub fn decompress_parallel(data: &[u8]) -> Result<Vec<u8>, BmrcError> {
    if data.len() < 8 {
        return Err(BmrcError::HeaderTooShort);
    }
    if data[..4] != PARALLEL_MAGIC {
        return Err(BmrcError::BadMagic);
    }

    let n = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;

    let sizes_end = 8 + 4 * n;
    if data.len() < sizes_end {
        return Err(BmrcError::Truncated);
    }

    let mut block_sizes = Vec::with_capacity(n);
    for i in 0..n {
        let off = 8 + i * 4;
        let size = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        block_sizes.push(size);
    }

    let mut offset = sizes_end;
    let mut slices: Vec<&[u8]> = Vec::with_capacity(n);
    for &size in &block_sizes {
        let end = offset.checked_add(size).ok_or(BmrcError::Truncated)?;
        if end > data.len() {
            return Err(BmrcError::Truncated);
        }
        slices.push(&data[offset..end]);
        offset = end;
    }

    let results: Vec<Result<Vec<u8>, BmrcError>> = thread::scope(|s| {
        let handles: Vec<_> = slices
            .iter()
            .map(|block| s.spawn(move || decompress(block)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("decompressor thread panicked"))
            .collect()
    });

    let mut out = Vec::new();
    for result in results {
        out.extend_from_slice(&result?);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8], level: u8, block_size: Option<usize>) {
        let compressed = compress_parallel(data, level, block_size);
        let restored = decompress_parallel(&compressed).expect("decompress_parallel failed");
        assert_eq!(restored, data);
    }

    #[test]
    fn empty_input() {
        roundtrip(b"", 5, None);
    }

    #[test]
    fn single_byte() {
        roundtrip(b"X", 3, None);
    }

    #[test]
    fn exactly_one_block() {
        let data = b"hello world".repeat(100);
        roundtrip(&data, 4, Some(data.len() + 1));
    }

    #[test]
    fn multiple_blocks() {
        let data = b"the quick brown fox jumps over the lazy dog. ".repeat(500);
        roundtrip(&data, 5, Some(1024));
    }

    #[test]
    fn block_boundary_exact() {
        // Data length is an exact multiple of block_size.
        let data = b"ABCD".repeat(256);
        roundtrip(&data, 3, Some(512));
    }

    #[test]
    fn block_size_one() {
        // Degenerate: every byte is its own block.
        roundtrip(b"abc", 1, Some(1));
    }

    #[test]
    fn all_levels_roundtrip() {
        let data = b"BMRC parallel compression test data -- ".repeat(200);
        for level in 1..=10u8 {
            roundtrip(&data, level, Some(8 * 1024));
        }
    }

    #[test]
    fn bad_magic_rejected() {
        let err = decompress_parallel(b"XXXX\x01\x00\x00\x00").unwrap_err();
        assert_eq!(err, BmrcError::BadMagic);
    }

    #[test]
    fn too_short_rejected() {
        let err = decompress_parallel(b"BMR").unwrap_err();
        assert_eq!(err, BmrcError::HeaderTooShort);
    }

    #[test]
    fn truncated_block_table_rejected() {
        // Claims 10 blocks but provides no size table.
        let mut bad = Vec::new();
        bad.extend_from_slice(&PARALLEL_MAGIC);
        bad.extend_from_slice(&10u32.to_le_bytes());
        let err = decompress_parallel(&bad).unwrap_err();
        assert_eq!(err, BmrcError::Truncated);
    }
}
