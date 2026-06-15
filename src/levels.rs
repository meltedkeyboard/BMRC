//! Compression level settings.
//!
//! BMRC supports compression levels from 1 to 10.
//! Lower levels are faster but usually compress less.
//! Higher levels use more memory and CPU time, but often
//! produce smaller output files.
//!
//! Each level changes which models are used and how they
//! are configured.

#[derive(Clone, Debug)]
pub struct LevelConfig {
    pub level: u8,
    pub orders: Vec<usize>,
    pub hash_bits: u32,
    pub match_model: bool,
    pub match_min: usize,
    pub match_hash_bits: u32,
    pub apm_stages: u8,
    pub mixer_contexts: usize,
    pub learning_rate: f64,
    pub update_rate: i32,
}
pub fn config_for_level(level: u8) -> LevelConfig {
    let level = level.clamp(1, 10);
    match level {
        1 => LevelConfig {
            level,
            orders: vec![1, 2],
            hash_bits: 16,
            match_model: false,
            match_min: 4,
            match_hash_bits: 16,
            apm_stages: 0,
            mixer_contexts: 1,
            learning_rate: 0.0015,
            update_rate: 5,
        },
        2 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3],
            hash_bits: 17,
            match_model: false,
            match_min: 4,
            match_hash_bits: 16,
            apm_stages: 0,
            mixer_contexts: 1,
            learning_rate: 0.0017,
            update_rate: 5,
        },
        3 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4],
            hash_bits: 18,
            match_model: true,
            match_min: 6,
            match_hash_bits: 16,
            apm_stages: 0,
            mixer_contexts: 2,
            learning_rate: 0.002,
            update_rate: 5,
        },
        4 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4, 6],
            hash_bits: 18,
            match_model: true,
            match_min: 5,
            match_hash_bits: 17,
            apm_stages: 1,
            mixer_contexts: 2,
            learning_rate: 0.002,
            update_rate: 5,
        },
        5 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4, 5, 6],
            hash_bits: 19,
            match_model: true,
            match_min: 5,
            match_hash_bits: 18,
            apm_stages: 1,
            mixer_contexts: 4,
            learning_rate: 0.0022,
            update_rate: 4,
        },
        6 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4, 5, 6, 8],
            hash_bits: 20,
            match_model: true,
            match_min: 4,
            match_hash_bits: 18,
            apm_stages: 1,
            mixer_contexts: 4,
            learning_rate: 0.0025,
            update_rate: 4,
        },
        7 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4, 5, 6, 8, 10],
            hash_bits: 21,
            match_model: true,
            match_min: 4,
            match_hash_bits: 19,
            apm_stages: 2,
            mixer_contexts: 8,
            learning_rate: 0.0025,
            update_rate: 4,
        },
        8 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4, 5, 6, 8, 10, 12],
            hash_bits: 21,
            match_model: true,
            match_min: 4,
            match_hash_bits: 20,
            apm_stages: 2,
            mixer_contexts: 8,
            learning_rate: 0.003,
            update_rate: 4,
        },
        9 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16],
            hash_bits: 22,
            match_model: true,
            match_min: 3,
            match_hash_bits: 21,
            apm_stages: 2,
            mixer_contexts: 16,
            learning_rate: 0.003,
            update_rate: 3,
        },
        10 => LevelConfig {
            level,
            orders: vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16, 24],
            hash_bits: 22,
            match_model: true,
            match_min: 3,
            match_hash_bits: 22,
            apm_stages: 2,
            mixer_contexts: 16,
            learning_rate: 0.0035,
            update_rate: 3,
        },
        _ => unreachable!("level clamped to 1..=10"),
    }
}

/// Returns the approximate memory usage for `level` in bytes
pub fn estimated_memory_bytes(level: u8) -> usize {
    let cfg = config_for_level(level);
    let table_bytes = (1usize << cfg.hash_bits) * 2; // u16 per entry
    let mut total = table_bytes * cfg.orders.len();
    if cfg.match_model {
        total += (1usize << cfg.match_hash_bits) * 4; // u32 per entry
    }
    if cfg.apm_stages >= 1 {
        total += 256 * 33 * 2;
    }
    if cfg.apm_stages >= 2 {
        total += 256 * 33 * 2;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_levels_valid() {
        for level in 1..=10u8 {
            let cfg = config_for_level(level);
            assert_eq!(cfg.level, level);
            assert!(!cfg.orders.is_empty());
            assert!(cfg.hash_bits >= 16);
            assert!(cfg.mixer_contexts >= 1);
        }
    }

    #[test]
    fn memory_grows_with_level() {
        let mut prev = 0;
        for level in 1..=10u8 {
            let mem = estimated_memory_bytes(level);
            assert!(mem >= prev, "memory should not shrink as level increases");
            prev = mem;
        }
    }

    #[test]
    fn out_of_range_clamped() {
        assert_eq!(config_for_level(0).level, 1);
        assert_eq!(config_for_level(255).level, 10);
    }
}
