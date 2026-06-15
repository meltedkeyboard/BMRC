//! Adaptive Context Mixer (ACM).
//!
//! The [`Predictor`] creates a probability for every bit.
//!
//! It gets predictions from several context models and from the
//! long-range match model. These predictions are combined by a mixer.
//! The result can then be refined by one or two SSE stages.
//!
//! The final probability is passed to the [`crate::range_coder`].
//! The decoder runs the same [`Predictor`] while reading the data,
//! which allows it to restore the original bytes.

use crate::levels::{config_for_level, LevelConfig};

/// Scale used for probabilities passed to the range coder
const PSCALE: f64 = 4096.0;

/// Converts a probability `p` from `(0, 1)` to the logistic
/// ("stretch") domain
#[inline]
fn stretch(p: f64) -> f64 {
    let p = p.clamp(1e-6, 1.0 - 1e-6);
    (p / (1.0 - p)).ln()
}

/// Converts a value from the logistic domain back to a probability
/// in `(0, 1)`
#[inline]
fn squash(x: f64) -> f64 {
    let x = x.clamp(-30.0, 30.0);
    1.0 / (1.0 + (-x).exp())
}

/// FNV-1a style hash over a byte slice
#[inline]
fn hash_slice(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0001_0000_01b3);
    }
    h
}

#[inline]
fn table_index(ctx_hash: u64, node: usize, mask: usize) -> usize {
    let h = ctx_hash.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (node as u64).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    ((h ^ (h >> 29)) as usize) & mask
}

#[inline]
fn match_bit_prediction(node: usize, predicted_byte: u8) -> Option<u8> {
    let bit_len = (usize::BITS - node.leading_zeros()) as usize; // 1..=8
    if bit_len == 0 || bit_len > 8 {
        return None;
    }
    let k = bit_len - 1; // bits already coded: 0..=7
    let actual_prefix = node & ((1usize << k) - 1);
    let predicted_prefix = (predicted_byte as usize) >> (8 - k);
    if actual_prefix == predicted_prefix {
        Some((predicted_byte >> (7 - k)) & 1)
    } else {
        None
    }
}

struct MatchModel {
    table: Vec<u32>,
    mask: usize,
    min_len: usize,
    ptr: usize,
    len: u32,
}

impl MatchModel {
    fn new(hash_bits: u32, min_len: usize) -> Self {
        MatchModel {
            table: vec![0u32; 1usize << hash_bits],
            mask: (1usize << hash_bits) - 1,
            min_len,
            ptr: 0,
            len: 0,
        }
    }

    fn start_byte(&mut self, history: &[u8]) {
        let n = history.len();
        if self.len == 0 && n >= self.min_len {
            let h = (hash_slice(&history[n - self.min_len..n]) as usize) & self.mask;
            let cand = self.table[h];
            if cand > 0 {
                let p = cand as usize;
                if p < n {
                    self.ptr = p;
                    self.len = self.min_len as u32;
                }
            }
        }
    }
    fn predicted(&self, history: &[u8]) -> Option<u8> {
        if self.len > 0 && self.ptr < history.len() {
            Some(history[self.ptr])
        } else {
            None
        }
    }
    fn confidence(&self) -> u32 {
        self.len.min(28)
    }
    fn end_byte(&mut self, history: &[u8], predicted: Option<u8>, actual: u8) {
        match predicted {
            Some(p) if p == actual => {
                self.len = self.len.saturating_add(1).min(1 << 20);
                self.ptr += 1;
            }
            _ => {
                self.len = 0;
            }
        }
        let n = history.len();
        if n >= self.min_len {
            let h = (hash_slice(&history[n - self.min_len..n]) as usize) & self.mask;
            self.table[h] = n as u32;
        }
    }
}

/// Adaptive Probability Map (a.k.a. Secondary Symbol Estimation / SSE).
///
/// Refines a probability estimate using a small additional context by
/// interpolating between 33 adaptively-trained anchor points spanning
/// the logistic range `[-8, 8]`.
struct Apm {
    table: Vec<u16>,
}

const APM_POINTS: usize = 33;

impl Apm {
    fn new(n_ctx: usize) -> Self {
        let mut table = vec![0u16; n_ctx * APM_POINTS];
        for c in 0..n_ctx {
            for i in 0..APM_POINTS {
                let x = (i as f64 - 16.0) / 2.0;
                let p = squash(x);
                table[c * APM_POINTS + i] = (p * 65535.0) as u16;
            }
        }
        Apm { table }
    }

    /// Returns the refined probability plus the two table indices that
    /// should be updated once the true bit is known.
    fn refine(&self, p: f64, ctx: usize) -> (f64, usize, usize) {
        let s = stretch(p).clamp(-7.999, 7.999);
        let pos = (s + 8.0) * 2.0; // 0..31.99
        let lo = pos.floor() as usize;
        let hi = lo + 1;
        let w = pos - lo as f64;
        let base = ctx * APM_POINTS;
        let plo = self.table[base + lo] as f64 / 65535.0;
        let phi = self.table[base + hi] as f64 / 65535.0;
        (plo * (1.0 - w) + phi * w, base + lo, base + hi)
    }

    fn update(&mut self, lo: usize, hi: usize, bit: u8) {
        const RATE: i32 = 6;
        let target = (bit as i32) * 65535;
        let a = self.table[lo] as i32;
        self.table[lo] = (a + ((target - a) >> RATE)).clamp(0, 65535) as u16;
        let b = self.table[hi] as i32;
        self.table[hi] = (b + ((target - b) >> RATE)).clamp(0, 65535) as u16;
    }
}

/// The full BMRC bit predictor for a chosen compression level.
///
/// A `Predictor` is driven one bit at a time:
///
/// ```text
/// predictor.start_byte();
/// for each of the 8 bits (MSB first):
///     p = predictor.predict();      // probability bit == 1, scaled 1..4095
///     // encode or decode `bit` using `p`
///     predictor.update(bit);
/// predictor.end_byte(byte_value);
/// ```
///
/// Both the compressor and decompressor run this exact sequence, so the
/// model stays perfectly synchronized
pub struct Predictor {
    cfg: LevelConfig,
    history: Vec<u8>,
    order_masks: Vec<usize>,
    tables: Vec<Vec<u16>>,
    ctx_hashes: Vec<u64>,
    weights: Vec<Vec<f64>>,
    match_model: Option<MatchModel>,
    apm1: Option<Apm>,
    apm2: Option<Apm>,
    node: usize,
    predicted_byte: Option<u8>,

    // Scratch state shared between predict() and update().
    cur_indices: Vec<usize>,
    cur_inputs: Vec<f64>,
    cur_mixer_ctx: usize,
    cur_mix_p: f64,
    cur_apm1_idx: (usize, usize),
    cur_apm2_idx: (usize, usize),
}

impl Predictor {
    /// Create a new predictor configured for `level` (clamped to
    /// `1..=10`).
    pub fn new(level: u8) -> Self {
        let cfg = config_for_level(level);
        let n_orders = cfg.orders.len();
        let table_size = 1usize << cfg.hash_bits;
        let tables = (0..n_orders).map(|_| vec![32768u16; table_size]).collect();
        let order_masks = vec![table_size - 1; n_orders];
        let n_inputs = n_orders + 2; // + match model + bias

        let weights = (0..cfg.mixer_contexts.max(1))
            .map(|_| vec![0.2f64; n_inputs])
            .collect();

        let match_model = if cfg.match_model {
            Some(MatchModel::new(cfg.match_hash_bits, cfg.match_min))
        } else {
            None
        };

        let apm1 = if cfg.apm_stages >= 1 {
            Some(Apm::new(256))
        } else {
            None
        };
        let apm2 = if cfg.apm_stages >= 2 {
            Some(Apm::new(256))
        } else {
            None
        };

        Predictor {
            cfg,
            history: Vec::new(),
            order_masks,
            tables,
            ctx_hashes: vec![0u64; n_orders],
            weights,
            match_model,
            apm1,
            apm2,
            node: 1,
            predicted_byte: None,
            cur_indices: vec![0; n_orders],
            cur_inputs: vec![0.0; n_inputs],
            cur_mixer_ctx: 0,
            cur_mix_p: 0.5,
            cur_apm1_idx: (0, 0),
            cur_apm2_idx: (0, 0),
        }
    }

    /// Prepare internal state for predicting the bits of the next byte
    pub fn start_byte(&mut self) {
        self.node = 1;
        let n = self.history.len();

        for (i, &order) in self.cfg.orders.iter().enumerate() {
            self.ctx_hashes[i] = if order == 0 {
                0
            } else if n >= order {
                hash_slice(&self.history[n - order..n])
            } else {
                // Not enough history yet: still produce a stable,
                // order-dependent hash so different orders don't
                // collide while warming up.
                hash_slice(&self.history[..n]) ^ (order as u64).wrapping_mul(0x9E3779B97F4A7C15)
            };
        }

        if let Some(mm) = self.match_model.as_mut() {
            mm.start_byte(&self.history);
            self.predicted_byte = mm.predicted(&self.history);
        } else {
            self.predicted_byte = None;
        }
    }

    pub fn predict(&mut self) -> u32 {
        let n_orders = self.cfg.orders.len();

        for i in 0..n_orders {
            let idx = table_index(self.ctx_hashes[i], self.node, self.order_masks[i]);
            self.cur_indices[i] = idx;
            let p = self.tables[i][idx] as f64 / 65536.0;
            self.cur_inputs[i] = stretch(p);
        }

        // Match-model input.
        let match_input = match (&self.match_model, self.predicted_byte) {
            (Some(mm), Some(pb)) => match match_bit_prediction(self.node, pb) {
                Some(bit) => {
                    let conf = mm.confidence() as f64;
                    if bit == 1 {
                        conf * 0.4
                    } else {
                        -conf * 0.4
                    }
                }
                None => 0.0,
            },
            _ => 0.0,
        };
        self.cur_inputs[n_orders] = match_input;
        self.cur_inputs[n_orders + 1] = 1.0; // bias term

        // Select a mixer weight set based on a small secondary context.
        let prev_byte = *self.history.last().unwrap_or(&0) as usize;
        self.cur_mixer_ctx = (prev_byte ^ self.node) % self.weights.len();

        let dot: f64 = self.weights[self.cur_mixer_ctx]
            .iter()
            .zip(self.cur_inputs.iter())
            .map(|(w, x)| w * x)
            .sum();
        let mut p = squash(dot);
        self.cur_mix_p = p;

        if let Some(apm) = &self.apm1 {
            let ctx = prev_byte & 0xFF;
            let (np, lo, hi) = apm.refine(p, ctx);
            self.cur_apm1_idx = (lo, hi);
            p = (p + 3.0 * np) / 4.0;
        }
        if let Some(apm) = &self.apm2 {
            let ctx = self.node & 0xFF;
            let (np, lo, hi) = apm.refine(p, ctx);
            self.cur_apm2_idx = (lo, hi);
            p = (p + 3.0 * np) / 4.0;
        }

        let p12 = (p * PSCALE).round() as i32;
        p12.clamp(1, 4095) as u32
    }

    /// Update all models after the true value of the bit just predicted
    /// becomes known
    pub fn update(&mut self, bit: u8) {
        let target16 = (bit as i32) * 65535;
        let rate = self.cfg.update_rate;

        for i in 0..self.cfg.orders.len() {
            let idx = self.cur_indices[i];
            let cur = self.tables[i][idx] as i32;
            let updated = cur + ((target16 - cur) >> rate);
            self.tables[i][idx] = updated.clamp(1, 65535) as u16;
        }

        // Mixer weight update (online gradient descent on log loss).
        let error = (bit as f64) - self.cur_mix_p;
        let lr = self.cfg.learning_rate;
        let ctx = self.cur_mixer_ctx;
        for (w, x) in self.weights[ctx].iter_mut().zip(self.cur_inputs.iter()) {
            *w += lr * error * x;
        }

        if let Some(apm) = self.apm1.as_mut() {
            let (lo, hi) = self.cur_apm1_idx;
            apm.update(lo, hi, bit);
        }
        if let Some(apm) = self.apm2.as_mut() {
            let (lo, hi) = self.cur_apm2_idx;
            apm.update(lo, hi, bit);
        }

        self.node = (self.node << 1) | (bit as usize);
    }

    /// Finalize a byte: append it to history and update the match model
    pub fn end_byte(&mut self, byte: u8) {
        let predicted = self.predicted_byte;
        self.history.push(byte);
        if let Some(mm) = self.match_model.as_mut() {
            mm.end_byte(&self.history, predicted, byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range_coder::{Decoder, Encoder};

    fn roundtrip(data: &[u8], level: u8) -> Vec<u8> {
        let mut enc = Encoder::new();
        let mut pred = Predictor::new(level);
        for &byte in data {
            pred.start_byte();
            for i in (0..8).rev() {
                let bit = (byte >> i) & 1;
                let p = pred.predict();
                enc.encode_bit(bit, p);
                pred.update(bit);
            }
            pred.end_byte(byte);
        }
        let stream = enc.finish();

        let mut dec = Decoder::new(&stream);
        let mut pred = Predictor::new(level);
        let mut out = Vec::with_capacity(data.len());
        for _ in 0..data.len() {
            pred.start_byte();
            let mut byte = 0u8;
            for _ in 0..8 {
                let p = pred.predict();
                let bit = dec.decode_bit(p);
                pred.update(bit);
                byte = (byte << 1) | bit;
            }
            pred.end_byte(byte);
            out.push(byte);
        }
        out
    }

    #[test]
    fn predictor_roundtrip_all_levels() {
        let data = b"the quick brown fox jumps over the lazy dog 0123456789 \
                      the quick brown fox jumps over the lazy dog 0123456789"
            .to_vec();
        for level in 1..=10u8 {
            let out = roundtrip(&data, level);
            assert_eq!(out, data, "mismatch at level {level}");
        }
    }

    #[test]
    fn stretch_squash_inverse() {
        for i in 1..1000 {
            let p = i as f64 / 1000.0;
            let s = stretch(p);
            let back = squash(s);
            assert!((p - back).abs() < 1e-6);
        }
    }

    #[test]
    fn match_prediction_prefix() {
        // node = 1 (no bits coded yet): predicts MSB of the byte.
        assert_eq!(match_bit_prediction(1, 0b1010_0000), Some(1));
        assert_eq!(match_bit_prediction(1, 0b0010_0000), Some(0));

        // After coding a "1" bit (node = 0b11 = 3), prefix is "1".
        assert_eq!(match_bit_prediction(0b11, 0b1010_0000), Some(0));
        // After coding a "0" bit (node = 0b10 = 2), predicted byte with
        // MSB=1 no longer matches the prefix.
        assert_eq!(match_bit_prediction(0b10, 0b1010_0000), None);
    }
}
