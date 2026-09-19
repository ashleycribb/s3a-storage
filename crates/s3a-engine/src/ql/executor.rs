use crate::ql::ast::*;
use crate::{
    MmapReader, QuerySieve, S3ACrudEngine, TileFusionEngine,
    TelemetryRecord, RoboticsKinematicRecord, GISSurveyPointRecord, EmbeddingRecord128, DACommitmentRecord,
    LearningActivityRecord, S3ACoordinate, TileType,
};
use s3a_simd::{cosine_similarity, dot_product};


#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    Telemetry(TelemetryRecord),
    TelemetryList(Vec<(TelemetryRecord, S3ACoordinate)>),
    KinematicList(Vec<(RoboticsKinematicRecord, S3ACoordinate)>),
    GisList(Vec<(GISSurveyPointRecord, S3ACoordinate)>),
    EmbeddingList(Vec<(EmbeddingRecord128, f32, S3ACoordinate)>),
    DaList(Vec<(DACommitmentRecord, S3ACoordinate)>),
    LearningActivityList(Vec<(LearningActivityRecord, S3ACoordinate)>),
    Inserted { coordinate: S3ACoordinate },
    Deleted { success: bool },
    Compacted { new_tile_count: u32 },
    Fused { new_tile_count: u32 },
}

pub struct S3AQLEngine;

impl S3AQLEngine {
    pub fn execute(stmt: &S3AStatement) -> Result<QueryResult, String> {
        match stmt {
            S3AStatement::Fetch(fetch) => Self::execute_fetch(fetch),
            S3AStatement::Sift(sift) => Self::execute_sift(sift),
            S3AStatement::Insert(insert) => Self::execute_insert(insert),
            S3AStatement::Delete(del) => Self::execute_delete(del),
            S3AStatement::Compact(comp) => Self::execute_compact(comp),
            S3AStatement::Fuse(fuse) => Self::execute_fuse(fuse),
        }
    }

    fn execute_fetch(fetch: &FetchStatement) -> Result<QueryResult, String> {
        let reader = MmapReader::open(&fetch.from_path)
            .map_err(|e| format!("Failed to open archive '{}': {}", fetch.from_path, e))?;

        let record: TelemetryRecord = reader.read_record_by_coordinate(&fetch.coordinate)
            .map_err(|e| format!("Failed to read record at coordinate {}: {}", fetch.coordinate, e))?;

        Ok(QueryResult::Telemetry(record))
    }

    fn execute_sift(sift: &SiftStatement) -> Result<QueryResult, String> {
        let reader = MmapReader::open(&sift.from_path)
            .map_err(|e| format!("Failed to open archive '{}': {}", sift.from_path, e))?;

        let sieve = QuerySieve::new(&reader);

        match sift.domain {
            SiftDomain::Telemetry => {
                let mut min_ts = 0u64;
                let mut max_ts = u64::MAX;
                let mut sensor_id = None;
                let mut metric_id = None;

                for cond in &sift.conditions {
                    match cond {
                        Condition::TimeBetween(min, max) => { min_ts = *min; max_ts = *max; }
                        Condition::SensorId(sid) => { sensor_id = Some(*sid); }
                        Condition::MetricId(mid) => { metric_id = Some(*mid); }
                        _ => {}
                    }
                }

                let records = sieve.query_telemetry_with_coords(min_ts, max_ts, sensor_id, metric_id);
                let limited = match sift.limit {
                    Some(lim) => records.into_iter().take(lim).collect(),
                    None => records,
                };
                Ok(QueryResult::TelemetryList(limited))
            }
            SiftDomain::Kinematics => {
                let mut min_t = 0u64;
                let mut max_t = u64::MAX;
                let mut min_xyz = [f32::NEG_INFINITY; 3];
                let mut max_xyz = [f32::INFINITY; 3];

                for cond in &sift.conditions {
                    match cond {
                        Condition::TimeBetween(min, max) => { min_t = *min; max_t = *max; }
                        Condition::SpatialBoxSimplex { min, max } => { min_xyz = *min; max_xyz = *max; }
                        _ => {}
                    }
                }

                let mut results = Vec::new();
                for i in 0..reader.tile_count() as usize {
                    if let Some((header, payload)) = reader.get_tile(i) {
                        if header.tile_type == TileType::ROBOTICS_KINEMATIC {
                            let recs: &[RoboticsKinematicRecord] = bytemuck::cast_slice(payload);
                            for (offset, rec) in recs.iter().enumerate() {
                                if rec.timestamp_us >= min_t && rec.timestamp_us <= max_t {
                                    let mut inside = true;
                                    for d in 0..3 {
                                        if rec.position_xyz[d] < min_xyz[d] || rec.position_xyz[d] > max_xyz[d] {
                                            inside = false;
                                            break;
                                        }
                                    }
                                    if inside {
                                        results.push((*rec, S3ACoordinate::new(0, i as u32, offset as u32)));
                                        if let Some(lim) = sift.limit {
                                            if results.len() >= lim {
                                                return Ok(QueryResult::KinematicList(results));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(QueryResult::KinematicList(results))
            }
            SiftDomain::GisMesh => {
                let mut min_lat = -90.0f64;
                let mut max_lat = 90.0f64;
                let mut min_lon = -180.0f64;
                let mut max_lon = 180.0f64;
                let mut min_elev = -100_000.0f64;
                let mut max_elev = 100_000.0f64;

                for cond in &sift.conditions {
                    match cond {
                        Condition::GisLatitudeBetween(min, max) => { min_lat = *min; max_lat = *max; }
                        Condition::GisLongitudeBetween(min, max) => { min_lon = *min; max_lon = *max; }
                        Condition::GisElevationBetween(min, max) => { min_elev = *min; max_elev = *max; }
                        _ => {}
                    }
                }

                let mut results = Vec::new();
                for i in 0..reader.tile_count() as usize {
                    if let Some((header, payload)) = reader.get_tile(i) {
                        if header.tile_type == TileType::GIS_SURVEY_MESH {
                            let recs: &[GISSurveyPointRecord] = bytemuck::cast_slice(payload);
                            for (offset, rec) in recs.iter().enumerate() {
                                let lat = rec.latitude_deg();
                                let lon = rec.longitude_deg();
                                let elev = rec.elevation_m();
                                if lat >= min_lat && lat <= max_lat &&
                                   lon >= min_lon && lon <= max_lon &&
                                   elev >= min_elev && elev <= max_elev {
                                    results.push((*rec, S3ACoordinate::new(0, i as u32, offset as u32)));
                                    if let Some(lim) = sift.limit {
                                        if results.len() >= lim {
                                            return Ok(QueryResult::GisList(results));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(QueryResult::GisList(results))
            }
            SiftDomain::Embeddings => {
                let mut target_vec = None;
                let mut metric = SimilarityMetric::Cosine;
                let mut threshold = 0.0f32;
                let mut min_t = 0u64;
                let mut max_t = u64::MAX;
                let mut compose_opt = None;

                for cond in &sift.conditions {
                    match cond {
                        Condition::TimeBetween(min, max) => { min_t = *min; max_t = *max; }
                        Condition::Similarity { vector, metric: m, threshold: t } => {
                            target_vec = Some(vector);
                            metric = *m;
                            threshold = *t;
                        }
                        Condition::AlgebraicCompose { vector, vector_weight, spatial_target, spatial_weight, halflife_secs, time_weight } => {
                            compose_opt = Some((vector, *vector_weight, *spatial_target, *spatial_weight, *halflife_secs, *time_weight));
                        }
                        _ => {}
                    }
                }

                let mut results = Vec::new();

                if let Some((comp_vec, v_weight, sp_target, sp_weight, hl_secs, t_weight)) = compose_opt {
                    for i in 0..reader.tile_count() as usize {
                        if let Some((header, payload)) = reader.get_tile(i) {
                            if header.tile_type == TileType::EMBEDDING {
                                let recs: &[EmbeddingRecord128] = bytemuck::cast_slice(payload);
                                for (offset, rec) in recs.iter().enumerate() {
                                    if rec.timestamp >= min_t && rec.timestamp <= max_t {
                                        let sim = cosine_similarity(&rec.vector, comp_vec);
                                        let sp_score = if let Some(pt) = sp_target {
                                            let d2 = (rec.vector[0] - pt[0]).powi(2)
                                                + (rec.vector[1] - pt[1]).powi(2)
                                                + (rec.vector[2] - pt[2]).powi(2);
                                            1.0 / (1.0 + d2.sqrt())
                                        } else {
                                            1.0
                                        };
                                        let time_score = if let Some(hl) = hl_secs {
                                            let dt = (max_t.saturating_sub(rec.timestamp)) as f64;
                                            let lambda = 0.69314718 / hl.max(1.0);
                                            (-lambda * dt).exp() as f32
                                        } else {
                                            1.0
                                        };

                                        let composite_score = v_weight * sim + sp_weight * sp_score + t_weight * time_score;
                                        results.push((*rec, composite_score, S3ACoordinate::new(0, i as u32, offset as u32)));
                                    }
                                }
                            }
                        }
                    }
                    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal));
                    if let Some(lim) = sift.limit {
                        results.truncate(lim);
                    }
                } else if let Some(target) = target_vec {
                    for i in 0..reader.tile_count() as usize {
                        if let Some((header, payload)) = reader.get_tile(i) {
                            if header.tile_type == TileType::EMBEDDING {
                                let recs: &[EmbeddingRecord128] = bytemuck::cast_slice(payload);
                                for (offset, rec) in recs.iter().enumerate() {
                                    if rec.timestamp >= min_t && rec.timestamp <= max_t {
                                        let sim = match metric {
                                            SimilarityMetric::Cosine => cosine_similarity(&rec.vector, target),
                                            SimilarityMetric::DotProduct => dot_product(&rec.vector, target),
                                        };
                                        if sim >= threshold {
                                            results.push((*rec, sim, S3ACoordinate::new(0, i as u32, offset as u32)));
                                            if let Some(lim) = sift.limit {
                                                if results.len() >= lim {
                                                    return Ok(QueryResult::EmbeddingList(results));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(QueryResult::EmbeddingList(results))
            }
            SiftDomain::DaCommitments => {
                let mut min_height = 0u64;
                let mut max_height = u64::MAX;

                for cond in &sift.conditions {
                    if let Condition::BlockHeightBetween(min, max) = cond {
                        min_height = *min;
                        max_height = *max;
                    }
                }

                let mut results = Vec::new();
                for i in 0..reader.tile_count() as usize {
                    if let Some((header, payload)) = reader.get_tile(i) {
                        if header.tile_type == TileType::DATA_AVAILABILITY {
                            let recs: &[DACommitmentRecord] = bytemuck::cast_slice(payload);
                            for (offset, rec) in recs.iter().enumerate() {
                                if rec.block_height >= min_height && rec.block_height <= max_height {
                                    results.push((*rec, S3ACoordinate::new(0, i as u32, offset as u32)));
                                    if let Some(lim) = sift.limit {
                                        if results.len() >= lim {
                                            return Ok(QueryResult::DaList(results));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(QueryResult::DaList(results))
            }
            SiftDomain::LearningActivities => {
                let mut min_ts = 0u64;
                let mut max_ts = u64::MAX;
                let mut actor_id = None;
                let mut verb_id = None;

                for cond in &sift.conditions {
                    match cond {
                        Condition::TimeBetween(min, max) => {
                            min_ts = *min;
                            max_ts = *max;
                        }
                        Condition::ActorId(a) => actor_id = Some(*a),
                        Condition::VerbId(v) => verb_id = Some(*v),
                        _ => {}
                    }
                }

                let sieve = QuerySieve::new(&reader);
                let mut results = sieve.query_learning_activities(min_ts, max_ts, actor_id, verb_id);
                if let Some(lim) = sift.limit {
                    results.truncate(lim);
                }
                Ok(QueryResult::LearningActivityList(results))
            }
        }
    }

    fn execute_insert(insert: &InsertStatement) -> Result<QueryResult, String> {
        let engine = S3ACrudEngine::open_or_create(&insert.into_path)
            .map_err(|e| format!("Failed to open engine at '{}': {}", insert.into_path, e))?;

        let rec = TelemetryRecord::new(insert.timestamp, insert.sensor_id, insert.metric_id, insert.value);
        let tile_id = engine.create_telemetry(&[rec])
            .map_err(|e| format!("Failed to insert telemetry record: {}", e))?;

        let coordinate = S3ACoordinate::new(0, (tile_id.saturating_sub(1)) as u32, 0);
        Ok(QueryResult::Inserted { coordinate })
    }

    fn execute_delete(del: &DeleteStatement) -> Result<QueryResult, String> {
        let engine = S3ACrudEngine::open_or_create(&del.from_path)
            .map_err(|e| format!("Failed to open engine at '{}': {}", del.from_path, e))?;

        let success = engine.delete_telemetry(del.sensor_id, del.metric_id, del.timestamp)
            .map_err(|e| format!("Failed to delete telemetry record: {}", e))?;

        Ok(QueryResult::Deleted { success })
    }

    fn execute_compact(comp: &CompactStatement) -> Result<QueryResult, String> {
        let engine = S3ACrudEngine::open_or_create(&comp.archive_path)
            .map_err(|e| format!("Failed to open engine at '{}': {}", comp.archive_path, e))?;

        let new_tile_count = engine.purge_and_compact()
            .map_err(|e| format!("Failed to compact archive '{}': {}", comp.archive_path, e))?;

        Ok(QueryResult::Compacted { new_tile_count })
    }

    fn execute_fuse(fuse: &FuseStatement) -> Result<QueryResult, String> {
        let count = TileFusionEngine::fuse_telemetry_tiles(&fuse.input_paths, fuse.output_path.clone())
            .map_err(|e| format!("Failed to fuse tiles into '{}': {}", fuse.output_path, e))?;

        Ok(QueryResult::Fused { new_tile_count: count })

    }
}
