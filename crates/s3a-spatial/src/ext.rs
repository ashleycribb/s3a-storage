use crate::pack::SpatialTileType;
use crate::records::GeoposeBasicQuatRecord;
use s3a_engine::S3ACrudEngine;
use std::io;

pub trait SpatialCrudExt {
    /// Appends GeoPose Basic-Quaternion records as tile 33.
    ///
    /// Fails with `PermissionDenied` when the archive does not have the
    /// `ADDON_SPATIAL` bit enabled (the engine write gate enforces it).
    fn create_geopose_records(&self, records: &[GeoposeBasicQuatRecord]) -> io::Result<u64>;
}

impl SpatialCrudExt for S3ACrudEngine {
    fn create_geopose_records(&self, records: &[GeoposeBasicQuatRecord]) -> io::Result<u64> {
        self.append_tile_records(
            SpatialTileType::GEOPOSE_BASIC_QUAT,
            records,
            None,
            None,
            None,
        )
    }
}
