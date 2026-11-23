//! S3A CAN bus bridge (`s3a-can-bridge`).
//!
//! Decodes J1939 (including DM1 and BAM) and OBD-II Mode 03 frames, ingests
//! replayable logs, and maps decoded faults to `s3a-pack-to` fault-isolation
//! node identifiers through a checked-in mapping table. Unmapped faults
//! return `NoMatch`. The crate never guesses.
//!
//! All sample data in this crate is synthetic. It is not a vehicle capture,
//! not a Technical Order, and not ETIMS or OEM content.
//!
//! Tiles 47..=48, addon bit `ADDON_CAN` (`0x0200`).
//!
//! Acceptance tests in `tests/acceptance.rs` are written first and must fail
//! until the implementation lands. This file is a placeholder so the crate
//! is in the workspace. Do not edit those tests to make them pass.
//!
//! Live SocketCAN is feature-gated (`socketcan`) and is not a fallback.
