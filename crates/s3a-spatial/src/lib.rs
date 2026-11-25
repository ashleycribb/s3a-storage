//! S3A Spatial Computing Domain Pack (`s3a-spatial`).
//!
//! Tile 33: OGC GeoPose 1.0 Basic-Quaternion records (`geopose_basic_quat`, 128 B).
//!
//! Attribution: implements OGC GeoPose 1.0 (document 21-056r11, approved 2022-06-20,
//! published 2023-09-08) from the specification. The OGC document license permits
//! implementation provided notices are kept; the codec here is written from scratch in
//! Rust and copies no code from any OSCP / OpenARCloud repository.

pub mod codec;
pub mod ext;
pub mod pack;
pub mod records;

pub use codec::{
    decode_geopose_basic_quat, encode_geopose_basic_quat, DecodedGeopose, GeoposeError,
};
pub use ext::SpatialCrudExt;
pub use pack::{register_pack_types, SpatialPack, SpatialTileType};
pub use records::GeoposeBasicQuatRecord;
