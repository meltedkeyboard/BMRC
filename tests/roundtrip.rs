use bmrc::{compress, decompress};

fn check_roundtrip(data: &[u8], level: u8) {
    let compressed = compress(data, level);
    let restored = decompress(&compressed).expect("decompress should succeed");
    assert_eq!(restored, data, "roundtrip mismatch at level {level}");
}

#[test]
fn empty_all_levels() {
    for level in 1..=10u8 {
        check_roundtrip(b"", level);
    }
}

#[test]
fn single_byte_all_levels() {
    for level in 1..=10u8 {
        check_roundtrip(b"X", level);
    }
}

#[test]
fn repetitive_text_all_levels() {
    let data = b"the quick brown fox jumps over the lazy dog. ".repeat(200);
    for level in 1..=10u8 {
        let compressed = compress(&data, level);
        check_roundtrip(&data, level);
        assert!(
            compressed.len() < data.len(),
            "level {level} did not compress repetitive text ({} -> {})",
            data.len(),
            compressed.len()
        );
    }
}

#[test]
fn english_like_text_all_levels() {
    let data = include_str!("sample.txt").as_bytes();
    for level in [1u8, 3, 6, 10] {
        check_roundtrip(data, level);
    }
}

#[test]
fn binary_data_all_levels() {
    // Structured binary data with some repeating patterns.
    let mut data = Vec::new();
    for i in 0u32..2000 {
        data.extend_from_slice(&i.to_le_bytes());
        if i % 7 == 0 {
            data.push(0xFF);
        }
    }
    for level in [1u8, 5, 10] {
        check_roundtrip(&data, level);
    }
}

#[test]
fn pseudo_random_data_does_not_expand_much() {
    let mut data = vec![0u8; 8192];
    let mut x: u32 = 0x1234_5678;
    for b in data.iter_mut() {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        *b = (x >> 24) as u8;
    }
    for level in [1u8, 5, 10] {
        let compressed = compress(&data, level);
        check_roundtrip(&data, level);
        assert!(
            compressed.len() <= data.len() + bmrc::HEADER_LEN,
            "level {level}: random data expanded too much ({} -> {})",
            data.len(),
            compressed.len()
        );
    }
}

#[test]
fn level_increases_compression_on_text() {
    let data = include_str!("sample.txt").as_bytes();
    let c1 = compress(data, 1).len();
    let c10 = compress(data, 10).len();
    assert!(
        c10 <= c1,
        "level 10 ({c10}) should compress at least as well as level 1 ({c1})"
    );
}
