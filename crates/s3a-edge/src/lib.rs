//! S3A Edge Profile: #![no_std] heapless flash writer with power-loss safety and trust chain.
//!
//! Conforms to S3A-EDGE-PROFILE.md (Phase 1).

#![cfg_attr(not(feature = "std"), no_std)]

pub mod crc;
pub mod flash;
pub mod header;
pub mod merkle;
pub mod records;
pub mod timeseries;
pub mod trust;
pub mod writer;

pub use crc::*;
pub use flash::*;
pub use header::*;
pub use records::*;
pub use timeseries::*;
pub use trust::*;
pub use writer::*;
