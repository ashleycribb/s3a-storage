use s3a_core::S3ACoordinate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiftDomain {
    Telemetry,
    Kinematics,
    GisMesh,
    Embeddings,
    DaCommitments,
    LearningActivities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimilarityMetric {
    Cosine,
    DotProduct,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    TimeBetween(u64, u64),
    SensorId(u32),
    MetricId(u32),
    ActorId(u64),
    VerbId(u32),
    SpatialBoxSimplex {
        min: [f32; 3],
        max: [f32; 3],
    },
    GisLatitudeBetween(f64, f64),
    GisLongitudeBetween(f64, f64),
    GisElevationBetween(f64, f64),
    Similarity {
        vector: Vec<f32>,
        metric: SimilarityMetric,
        threshold: f32,
    },
    AlgebraicCompose {
        vector: Vec<f32>,
        vector_weight: f32,
        spatial_target: Option<[f32; 3]>,
        spatial_weight: f32,
        halflife_secs: Option<f64>,
        time_weight: f32,
    },
    BlockHeightBetween(u64, u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchStatement {
    pub coordinate: S3ACoordinate,
    pub from_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SiftStatement {
    pub domain: SiftDomain,
    pub from_path: String,
    pub conditions: Vec<Condition>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertStatement {
    pub timestamp: u64,
    pub sensor_id: u32,
    pub metric_id: u32,
    pub value: f64,
    pub into_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteStatement {
    pub sensor_id: u32,
    pub metric_id: u32,
    pub timestamp: u64,
    pub from_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompactStatement {
    pub archive_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuseStatement {
    pub input_paths: Vec<String>,
    pub output_path: String,
    pub resolve_versioned_updates: bool,
    pub purge_tombstones: bool,
    pub recalculate_simplex_hulls: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum S3AStatement {
    Fetch(FetchStatement),
    Sift(SiftStatement),
    Insert(InsertStatement),
    Delete(DeleteStatement),
    Compact(CompactStatement),
    Fuse(FuseStatement),
}
