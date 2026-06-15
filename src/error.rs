//! returned by [`crate::decompress`]

use std::fmt;

/// errs that can happen while decompressing a BMRC stream
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BmrcError {
    /// input is too short
    HeaderTooShort,
    /// file does not start with the expected `"BMR1"` bytes
    BadMagic,
    /// input ends before all stored data is available
    Truncated,
}

impl fmt::Display for BmrcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BmrcError::HeaderTooShort => write!(f, "input too short to contain a BMRC header"),
            BmrcError::BadMagic => write!(f, "invalid BMRC magic bytes (not a .bmrc stream)"),
            BmrcError::Truncated => write!(f, "truncated BMRC stream"),
        }
    }
}

impl std::error::Error for BmrcError {}
