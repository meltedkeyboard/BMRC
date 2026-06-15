//! Binary range coder used as the final entropy-coding stage of BMRC.
//!
//! This is a 32-bit carry-less binary range coder in the style used by
//! `fpaq0` / `lpaq` family compressors. It accepts, for every bit, a
//! 12-bit probability `p1` (the probability that the bit equals `1`,
//! in the range `1..=4095`) and encodes/decodes that single bit.
//!
//! Probabilities come from the [`crate::predictor`]
//! module and are updated after every bit, so the range coder itself is
//! stateless with respect to symbol statistics - it only tracks the
//! current `[x1, x2)` interval.

const TOP_MASK: u32 = 0xFF00_0000;
const PSCALE_BITS: u32 = 12;

/// Range coder encoder. Produces a byte stream that can be fed into
/// [`Decoder`] together with the same sequence of probabilities to
/// recover the original bit sequence.
pub struct Encoder {
    x1: u32,
    x2: u32,
    out: Vec<u8>,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Encoder {
            x1: 0,
            x2: 0xFFFF_FFFF,
            out: Vec::new(),
        }
    }

    /// Encode a single bit given `p1`, the probability (scaled to
    /// `1..=4095`) that the bit is `1`.
    #[inline]
    pub fn encode_bit(&mut self, bit: u8, p1: u32) {
        debug_assert!(p1 >= 1 && p1 < (1 << PSCALE_BITS));
        let range = (self.x2 - self.x1) as u64;
        let xmid = self.x1 + ((range * p1 as u64) >> PSCALE_BITS) as u32;

        if bit != 0 {
            self.x2 = xmid;
        } else {
            self.x1 = xmid + 1;
        }

        while (self.x1 ^ self.x2) & TOP_MASK == 0 {
            self.out.push((self.x2 >> 24) as u8);
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 0xFF;
        }
    }

    /// Flush remaining state and return the encoded byte stream.
    pub fn finish(mut self) -> Vec<u8> {
        // Emit enough bytes of x1 to allow the decoder to disambiguate
        // the final interval.
        for _ in 0..4 {
            self.out.push((self.x1 >> 24) as u8);
            self.x1 <<= 8;
        }
        self.out
    }
}

/// Range coder decoder. Mirrors [`Encoder`] bit for bit: for every bit,
/// the caller must supply the same probability `p1` that was used during
/// encoding (derived from the same adaptive model running in lock-step).
pub struct Decoder<'a> {
    x1: u32,
    x2: u32,
    x: u32,
    input: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        let mut d = Decoder {
            x1: 0,
            x2: 0xFFFF_FFFF,
            x: 0,
            input,
            pos: 0,
        };
        for _ in 0..4 {
            d.x = (d.x << 8) | d.next_byte() as u32;
        }
        d
    }

    #[inline]
    fn next_byte(&mut self) -> u8 {
        let b = if self.pos < self.input.len() {
            self.input[self.pos]
        } else {
            0
        };
        self.pos += 1;
        b
    }

    /// Decode a single bit given `p1`, the probability (scaled to
    /// `1..=4095`) that the bit is `1`. Must be called with exactly the
    /// same sequence of probabilities used by [`Encoder::encode_bit`].
    #[inline]
    pub fn decode_bit(&mut self, p1: u32) -> u8 {
        debug_assert!(p1 >= 1 && p1 < (1 << PSCALE_BITS));
        let range = (self.x2 - self.x1) as u64;
        let xmid = self.x1 + ((range * p1 as u64) >> PSCALE_BITS) as u32;

        let bit = if self.x <= xmid { 1 } else { 0 };

        if bit != 0 {
            self.x2 = xmid;
        } else {
            self.x1 = xmid + 1;
        }

        while (self.x1 ^ self.x2) & TOP_MASK == 0 {
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 0xFF;
            self.x = (self.x << 8) | self.next_byte() as u32;
        }

        bit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_fixed_prob() {
        let bits: Vec<u8> = (0..10000).map(|i| ((i * 7 + 3) % 5 == 0) as u8).collect();
        let mut enc = Encoder::new();
        for &b in &bits {
            enc.encode_bit(b, 2048);
        }
        let data = enc.finish();

        let mut dec = Decoder::new(&data);
        for &b in &bits {
            assert_eq!(dec.decode_bit(2048), b);
        }
    }

    #[test]
    fn roundtrip_varied_prob() {
        let mut enc = Encoder::new();
        let mut bits = Vec::new();
        let mut p: u32 = 1;
        for i in 0..5000u32 {
            let bit = (i % 3 == 0) as u8;
            bits.push((bit, p));
            enc.encode_bit(bit, p);
            p = ((p * 37 + 11) % 4094) + 1;
        }
        let data = enc.finish();

        let mut dec = Decoder::new(&data);
        for (bit, p) in bits {
            assert_eq!(dec.decode_bit(p), bit);
        }
    }
}
