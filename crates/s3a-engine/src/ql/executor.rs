use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::ql::ast::*;
use crate::ql::error::QlError;
use crate::{
    EmbeddingRecord128, LearningActivityRecord, MmapReader, QuerySieve, S3ACoordinate,
    S3ACrudEngine, TelemetryRecord, TileFusionEngine, TileType,
};
use s3a_simd::{cosine_similarity, dot_product};

#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    Telemetry(TelemetryRecord),
    TelemetryList(Vec<(TelemetryRecord, S3ACoordinate)>),
    EmbeddingList(Vec<(EmbeddingRecord128, f32, S3ACoordinate)>),
    LearningActivityList(Vec<(LearningActivityRecord, S3ACoordinate)>),
    PackData {
        domain: SiftDomain,
        count: usize,
        summary: String,
        json_payload: Option<String>,
    },
    Inserted {
        coordinate: S3ACoordinate,
    },
    Deleted {
        success: bool,
    },
    Compacted {
        new_tile_count: u32,
    },
    Fused {
        new_tile_count: u32,
    },
}

pub type PackSiftHandler =
    Arc<dyn Fn(&SiftStatement) -> Result<QueryResult, QlError> + Send + Sync>;

static PACK_HANDLERS: OnceLock<RwLock<HashMap<SiftDomain, PackSiftHandler>>> = OnceLock::new();

fn get_pack_handlers() -> &'static RwLock<HashMap<SiftDomain, PackSiftHandler>> {
    PACK_HANDLERS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Registers a domain pack's SIFT query handler.
pub fn register_pack_sift_handler(domain: SiftDomain, handler: PackSiftHandler) {
    let mut map = get_pack_handlers().write().unwrap();
    map.insert(domain, handler);
}

pub struct S3AQLEngine;

impl S3AQLEngine {
    pub fn execute(stmt: &S3AStatement) -> Result<QueryResult, QlError> {
        match stmt {
            S3AStatement::Fetch(fetch) => Self::execute_fetch(fetch),
            S3AStatement::Sift(sift) => Self::execute_sift(sift),
            S3AStatement::Insert(insert) => Self::execute_insert(insert),
            S3AStatement::Delete(del) => Self::execute_delete(del),
            S3AStatement::Compact(comp) => Self::execute_compact(comp),
            S3AStatement::Fuse(fuse) => Self::execute_fuse(fuse),
        }
    }

    fn execute_fetch(fetch: &FetchStatement) -> Result<QueryResult, QlError> {
        let reader = MmapReader::open(&fetch.from_path).map_err(QlError::Io)?;

        let record: TelemetryRecord = reader
            .read_record_by_coordinate(&fetch.coordinate)
            .map_err(|_| QlError::NotFound {
                coordinate: fetch.coordinate.to_string(),
            })?;

        Ok(QueryResult::Telemetry(record))
    }

    fn execute_sift(sift: &SiftStatement) -> Result<QueryResult, QlError> {
        // First, check if this is a registered pack domain or an unhandled pack domain
        match sift.domain {
            SiftDomain::Kinematics => {
                let map = get_pack_handlers().read().unwrap();
                if let Some(handler) = map.get(&SiftDomain::Kinematics) {
                    return handler(sift);
                }
                return Err(QlError::PackRequired {
                    pack: "s3a-pack-robotics",
                    feature: "kinematics",
                });
            }
            SiftDomain::GisMesh => {
                let map = get_pack_handlers().read().unwrap();
                if let Some(handler) = map.get(&SiftDomain::GisMesh) {
                    return handler(sift);
                }
                return Err(QlError::PackRequired {
                    pack: "s3a-pack-gis",
                    feature: "gis_mesh",
                });
            }
            SiftDomain::DaCommitments => {
                let map = get_pack_handlers().read().unwrap();
                if let Some(handler) = map.get(&SiftDomain::DaCommitments) {
                    return handler(sift);
                }
                return Err(QlError::PackRequired {
                    pack: "s3a-pack-da",
                    feature: "da_commitments",
                });
            }
            SiftDomain::HumanLrs | SiftDomain::LearningActivities => {
                let map = get_pack_handlers().read().unwrap();
                if let Some(handler) = map.get(&sift.domain) {
                    return handler(sift);
                }
                // Also check fallback between HumanLrs and LearningActivities
                if let Some(handler) = map.get(&SiftDomain::HumanLrs) {
                    return handler(sift);
                }
                return Err(QlError::PackRequired {
                    pack: "s3a-pack-scholar",
                    feature: "human_lrs",
                });
            }
            SiftDomain::AiTraces => {
                let map = get_pack_handlers().read().unwrap();
                if let Some(handler) = map.get(&SiftDomain::AiTraces) {
                    return handler(sift);
                }
                return Err(QlError::PackRequired {
                    pack: "s3a-pack-scholar",
                    feature: "ai_traces",
                });
            }
            SiftDomain::AcademicPapers => {
                let map = get_pack_handlers().read().unwrap();
                if let Some(handler) = map.get(&SiftDomain::AcademicPapers) {
                    return handler(sift);
                }
                return Err(QlError::PackRequired {
                    pack: "s3a-pack-scholar",
                    feature: "academic_papers",
                });
            }
            SiftDomain::Telemetry => {
                let reader = MmapReader::open(&sift.from_path).map_err(QlError::Io)?;
                let sieve = QuerySieve::new(&reader);

                let mut min_ts = 0u64;
                let mut max_ts = u64::MAX;
                let mut sensor_id = None;
                let mut metric_id = None;

                for cond in &sift.conditions {
                    match cond {
                        Condition::TimeBetween(min, max) => {
                            min_ts = *min;
                            max_ts = *max;
                        }
                        Condition::SensorId(sid) => {
                            sensor_id = Some(*sid);
                        }
                        Condition::MetricId(mid) => {
                            metric_id = Some(*mid);
                        }
                        _ => {}
                    }
                }

                let records =
                    sieve.query_telemetry_with_coords(min_ts, max_ts, sensor_id, metric_id);
                let limited = match sift.limit {
                    Some(lim) => records.into_iter().take(lim).collect(),
                    None => records,
                };
                Ok(QueryResult::TelemetryList(limited))
            }
            SiftDomain::Embeddings => {
                let reader = MmapReader::open(&sift.from_path).map_err(QlError::Io)?;
                let mut target_vec = None;
                let mut metric = SimilarityMetric::Cosine;
                let mut threshold = 0.0f32;
                let mut min_t = 0u64;
                let mut max_t = u64::MAX;
                let mut compose_opt = None;

                for cond in &sift.conditions {
                    match cond {
                        Condition::TimeBetween(min, max) => {
                            min_t = *min;
                            max_t = *max;
                        }
                        Condition::Similarity {
                            vector,
                            metric: m,
                            threshold: t,
                        } => {
                            target_vec = Some(vector);
                            metric = *m;
                            threshold = *t;
                        }
                        Condition::AlgebraicCompose {
                            vector,
                            vector_weight,
                            spatial_target,
                            spatial_weight,
                            halflife_secs,
                            time_weight,
                        } => {
                            compose_opt = Some((
                                vector,
                                *vector_weight,
                                *spatial_target,
                                *spatial_weight,
                                *halflife_secs,
                                *time_weight,
                            ));
                        }
                        _ => {}
                    }
                }

                let mut results = Vec::new();

                if let Some((comp_vec, v_weight, sp_target, sp_weight, hl_secs, t_weight)) =
                    compose_opt
                {
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

                                        let composite_score = v_weight * sim
                                            + sp_weight * sp_score
                                            + t_weight * time_score;
                                        results.push((
                                            *rec,
                                            composite_score,
                                            S3ACoordinate::new(0, i as u32, offset as u32),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    results.sort_by(|a, b| {
                        b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal)
                    });
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
                                            SimilarityMetric::Cosine => {
                                                cosine_similarity(&rec.vector, target)
                                            }
                                            SimilarityMetric::DotProduct => {
                                                dot_product(&rec.vector, target)
                                            }
                                        };
                                        if sim >= threshold {
                                            results.push((
                                                *rec,
                                                sim,
                                                S3ACoordinate::new(0, i as u32, offset as u32),
                                            ));
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
        }
    }

    fn execute_insert(insert: &InsertStatement) -> Result<QueryResult, QlError> {
        let engine = S3ACrudEngine::open_or_create(&insert.into_path).map_err(QlError::Io)?;

        let rec = TelemetryRecord::new(
            insert.timestamp,
            insert.sensor_id,
            insert.metric_id,
            insert.value,
        );
        let tile_id = engine
            .create_telemetry(&[rec])
            .map_err(|e| QlError::Internal(e.to_string()))?;

        let coordinate = S3ACoordinate::new(0, (tile_id.saturating_sub(1)) as u32, 0);
        Ok(QueryResult::Inserted { coordinate })
    }

    fn execute_delete(del: &DeleteStatement) -> Result<QueryResult, QlError> {
        let engine = S3ACrudEngine::open_or_create(&del.from_path).map_err(QlError::Io)?;

        let success = engine
            .delete_telemetry(del.sensor_id, del.metric_id, del.timestamp)
            .map_err(|e| QlError::Internal(e.to_string()))?;

        Ok(QueryResult::Deleted { success })
    }

    fn execute_compact(comp: &CompactStatement) -> Result<QueryResult, QlError> {
        let engine = S3ACrudEngine::open_or_create(&comp.archive_path).map_err(QlError::Io)?;

        let new_tile_count = engine
            .purge_and_compact()
            .map_err(|e| QlError::Internal(e.to_string()))?;

        Ok(QueryResult::Compacted { new_tile_count })
    }

    fn execute_fuse(fuse: &FuseStatement) -> Result<QueryResult, QlError> {
        let count =
            TileFusionEngine::fuse_telemetry_tiles(&fuse.input_paths, fuse.output_path.clone())
                .map_err(|e| QlError::Internal(e.to_string()))?;

        Ok(QueryResult::Fused {
            new_tile_count: count,
        })
    }
}
