//! Burrows-Wheeler Transform (BWT) pre-pass.
//!
//! The BWT reorders bytes so that equal bytes tend to cluster together,
//! which improves prediction accuracy for the context models that follow.
//!
//! The encoder sorts all cyclic rotations of the input and emits the
//! last column of the sorted matrix, plus the index of the original
//! string in the sorted order (the "primary index").
//!
//! The decoder reconstructs the original string from the last column
//! and the primary index using the LF-mapping property.

/// Returns the BWT of `data` and the primary index needed for inversion.
///
/// The primary index identifies which row in the sorted rotation matrix
/// corresponds to the rotation starting at position 0 (the original string).
///
/// Uses a simple O(n log n) average rotation sort. For very large blocks a
/// linear-time suffix array (SA-IS) would be faster, but correctness is
/// identical.
pub fn bwt_encode(data: &[u8]) -> (Vec<u8>, u32) {
    let n = data.len();
    if n == 0 {
        return (Vec::new(), 0);
    }

    let mut sa: Vec<u32> = (0..n as u32).collect();
    sa.sort_unstable_by(|&a, &b| {
        let a = a as usize;
        let b = b as usize;
        for k in 0..n {
            let ca = data[(a + k) % n];
            let cb = data[(b + k) % n];
            if ca != cb {
                return ca.cmp(&cb);
            }
        }
        std::cmp::Ordering::Equal
    });

    let bwt: Vec<u8> = sa.iter().map(|&i| data[(i as usize + n - 1) % n]).collect();
    let primary_idx = sa.iter().position(|&i| i == 0).unwrap() as u32;
    (bwt, primary_idx)
}

/// Reconstructs the original byte string from its BWT and primary index.
///
/// Uses the LF-mapping: the i-th occurrence of byte `c` in the last column
/// maps to the i-th occurrence of `c` in the first column (sorted order).
pub fn bwt_decode(bwt: &[u8], primary_idx: u32) -> Vec<u8> {
    let n = bwt.len();
    if n == 0 {
        return Vec::new();
    }

    // Count occurrences of every byte value.
    let mut count = [0usize; 256];
    for &b in bwt {
        count[b as usize] += 1;
    }

    // Cumulative sum: first_col_start[c] = number of bytes with value < c.
    let mut first_col_start = [0usize; 256];
    let mut total = 0;
    for i in 0..256 {
        first_col_start[i] = total;
        total += count[i];
    }

    // Build the LF-mapping array.
    let mut occ = [0usize; 256];
    let mut lf = vec![0usize; n];
    for (i, &b) in bwt.iter().enumerate() {
        let b = b as usize;
        lf[i] = first_col_start[b] + occ[b];
        occ[b] += 1;
    }

    // Walk the LF-mapping backwards to recover the original string.
    let mut result = vec![0u8; n];
    let mut row = primary_idx as usize;
    for j in (0..n).rev() {
        result[j] = bwt[row];
        row = lf[row];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let (bwt, idx) = bwt_encode(data);
        let recovered = bwt_decode(&bwt, idx);
        assert_eq!(recovered, data);
    }

    #[test]
    fn empty() {
        roundtrip(b"");
    }

    #[test]
    fn single_byte() {
        roundtrip(b"A");
    }

    #[test]
    fn known_banana() {
        // "banana" -> BWT "nnbaaa", primary index 3
        let (bwt, idx) = bwt_encode(b"banana");
        assert_eq!(&bwt, b"nnbaaa");
        assert_eq!(idx, 3);
        roundtrip(b"banana");
    }

    #[test]
    fn all_same_bytes() {
        roundtrip(b"aaaaaaaaaa");
    }

    #[test]
    fn repetitive_text() {
        roundtrip(&b"the quick brown fox jumps over the lazy dog. ".repeat(20));
    }

    #[test]
    fn binary_data() {
        let data: Vec<u8> = (0u8..=255).cycle().take(512).collect();
        roundtrip(&data);
    }
}
