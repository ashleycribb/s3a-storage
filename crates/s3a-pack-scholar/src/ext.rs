use s3a_core::{S3ACoordinate, S3AError, HYPER_TILE_PAYLOAD_SIZE};
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine, TileWriter};
use s3a_simd::{can_reject_tile_bloom, can_reject_tile_time};
use std::io;
use std::path::Path;

use crate::pack::ScholarTileType;
use crate::records::{
    AcademicPaperRecord, AgentRegistryRecord, AiTraceRecord, CitationFingerprintRecord,
    HumanLrsRecord, ResearchGraphEdgeRecord, ScholarNoteClaimRecord, SrsManifestHeader,
    SrsWorkspaceRecord,
};
use crate::snapshot::{export_srs_snapshot, SrsSnapshotBundle};

/// High-level CRUD operations for the Scholar Explorer domain pack.
pub trait ScholarCrudExt {
    fn create_human_lrs(&self, records: &[HumanLrsRecord]) -> io::Result<u64>;
    fn create_ai_traces(&self, records: &[AiTraceRecord]) -> io::Result<u64>;
    fn create_academic_papers(&self, records: &[AcademicPaperRecord]) -> io::Result<u64>;
    fn create_research_graph_edges(&self, edges: &[ResearchGraphEdgeRecord]) -> io::Result<u64>;
    fn create_srs_workspace(&self, record: &SrsWorkspaceRecord) -> io::Result<u64>;
    fn create_agent_registry(&self, records: &[AgentRegistryRecord]) -> io::Result<u64>;
    fn create_note_claims(&self, records: &[ScholarNoteClaimRecord]) -> io::Result<u64>;
    fn create_citation_fingerprints(
        &self,
        records: &[CitationFingerprintRecord],
    ) -> io::Result<u64>;
    fn trace_ai_from_human(&self, human_coord: &S3ACoordinate) -> Result<AiTraceRecord, S3AError>;
    fn trace_human_from_ai(&self, ai_coord: &S3ACoordinate) -> Result<HumanLrsRecord, S3AError>;
    fn query_session_bundle(
        &self,
        session_uuid: [u64; 2],
    ) -> io::Result<(
        Vec<(HumanLrsRecord, S3ACoordinate)>,
        Vec<(AiTraceRecord, S3ACoordinate)>,
    )>;
    fn export_srs_snapshot<P: AsRef<Path>>(
        &self,
        dest_path: P,
        manifest: &SrsManifestHeader,
        workspace: Option<&SrsWorkspaceRecord>,
        project_uuid: Option<[u64; 2]>,
    ) -> io::Result<SrsSnapshotBundle>;
}

impl ScholarCrudExt for S3ACrudEngine {
    fn create_human_lrs(&self, records: &[HumanLrsRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();
        let mut filter_keys: Vec<u64> = Vec::with_capacity(records.len() * 2);
        for r in records {
            filter_keys.push(r.session_uuid[0] ^ r.session_uuid[1]);
            filter_keys.push(r.actor_hash);
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<HumanLrsRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records exceed single tile capacity",
            ));
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::HUMAN_LRS,
            records,
            Some(&timestamps),
            None,
            Some(&filter_keys),
        )
    }

    fn create_ai_traces(&self, records: &[AiTraceRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();
        let mut filter_keys: Vec<u64> = Vec::with_capacity(records.len() * 2);
        for r in records {
            filter_keys.push(r.session_uuid[0] ^ r.session_uuid[1]);
            filter_keys.push(r.agent_id);
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<AiTraceRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records exceed single tile capacity",
            ));
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::AI_TRACE_LOG,
            records,
            Some(&timestamps),
            None,
            Some(&filter_keys),
        )
    }

    fn create_academic_papers(&self, records: &[AcademicPaperRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();
        let mut filter_keys: Vec<u64> = Vec::with_capacity(records.len() * 2);
        for r in records {
            filter_keys.push(r.paper_id);
            filter_keys.push(r.doi_prefix_hash);
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<AcademicPaperRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records exceed single tile capacity",
            ));
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::ACADEMIC_PAPERS,
            records,
            Some(&timestamps),
            None,
            Some(&filter_keys),
        )
    }

    fn create_research_graph_edges(&self, edges: &[ResearchGraphEdgeRecord]) -> io::Result<u64> {
        if edges.is_empty() {
            return Ok(0);
        }
        let timestamps: Vec<u64> = edges.iter().map(|e| e.timestamp_sec).collect();
        let mut filter_keys: Vec<u64> = Vec::with_capacity(edges.len() * 2);
        for e in edges {
            filter_keys.push(e.subject_hash);
            filter_keys.push(e.object_hash);
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<ResearchGraphEdgeRecord>();
        if edges.len() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records exceed single tile capacity",
            ));
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::RESEARCH_GRAPH,
            edges,
            Some(&timestamps),
            None,
            Some(&filter_keys),
        )
    }

    fn create_srs_workspace(&self, record: &SrsWorkspaceRecord) -> io::Result<u64> {
        let filter_keys = [
            record.project_uuid[0] ^ record.project_uuid[1],
            record.question_hash,
        ];
        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::SRS_WORKSPACE,
            std::slice::from_ref(record),
            None,
            None,
            Some(&filter_keys),
        )
    }

    fn create_agent_registry(&self, records: &[AgentRegistryRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let mut filter_keys: Vec<u64> = Vec::with_capacity(records.len() * 3);
        for r in records {
            filter_keys.push(r.agent_id);
            filter_keys.push(r.xapi_profile_iri_hash);
            filter_keys.push(r.owl_ontology_iri_hash);
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<AgentRegistryRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records exceed single tile capacity",
            ));
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::AGENT_REGISTRY,
            records,
            None,
            None,
            Some(&filter_keys),
        )
    }

    fn create_note_claims(&self, records: &[ScholarNoteClaimRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();
        let mut filter_keys: Vec<u64> = Vec::with_capacity(records.len() * 3);
        for r in records {
            filter_keys.push(r.claim_id);
            filter_keys.push(r.session_uuid[0] ^ r.session_uuid[1]);
            filter_keys.push(r.author_hash);
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<ScholarNoteClaimRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records exceed single tile capacity",
            ));
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::NOTE_CLAIMS,
            records,
            Some(&timestamps),
            None,
            Some(&filter_keys),
        )
    }

    fn create_citation_fingerprints(
        &self,
        records: &[CitationFingerprintRecord],
    ) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let timestamps: Vec<u64> = records.iter().map(|r| r.fetched_at).collect();
        let mut filter_keys: Vec<u64> = Vec::with_capacity(records.len() * 2);
        for r in records {
            filter_keys.push(r.paper_id);
            filter_keys.push(r.doi_hash);
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<CitationFingerprintRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records exceed single tile capacity",
            ));
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            ScholarTileType::CITATION_FINGERPRINT,
            records,
            Some(&timestamps),
            None,
            Some(&filter_keys),
        )
    }

    fn trace_ai_from_human(&self, human_coord: &S3ACoordinate) -> Result<AiTraceRecord, S3AError> {
        let human: HumanLrsRecord = self.read_by_coordinate(human_coord)?;
        self.read_by_coordinate(&human.ai_trace_coord)
    }

    fn trace_human_from_ai(&self, ai_coord: &S3ACoordinate) -> Result<HumanLrsRecord, S3AError> {
        let ai: AiTraceRecord = self.read_by_coordinate(ai_coord)?;
        self.read_by_coordinate(&ai.human_coord)
    }

    fn query_session_bundle(
        &self,
        session_uuid: [u64; 2],
    ) -> io::Result<(
        Vec<(HumanLrsRecord, S3ACoordinate)>,
        Vec<(AiTraceRecord, S3ACoordinate)>,
    )> {
        let reader = MmapReader::open(self.file_path())?;
        let sieve = QuerySieve::new(&reader);
        let human_records = sieve.query_human_lrs(Some(session_uuid), None, 0, u64::MAX);
        let ai_records = sieve.query_ai_traces(Some(session_uuid), None, 0, u64::MAX);
        Ok((human_records, ai_records))
    }

    fn export_srs_snapshot<P: AsRef<Path>>(
        &self,
        dest_path: P,
        manifest: &SrsManifestHeader,
        workspace: Option<&SrsWorkspaceRecord>,
        project_uuid: Option<[u64; 2]>,
    ) -> io::Result<SrsSnapshotBundle> {
        export_srs_snapshot(
            self.file_path(),
            dest_path,
            manifest,
            workspace,
            project_uuid,
        )
    }
}

/// Query extension methods for searching Scholar Pack Hyper-Tiles via QuerySieve.
pub trait ScholarQueryExt {
    fn query_human_lrs(
        &self,
        session_uuid: Option<[u64; 2]>,
        actor_hash: Option<u64>,
        min_ts: u64,
        max_ts: u64,
    ) -> Vec<(HumanLrsRecord, S3ACoordinate)>;

    fn query_ai_traces(
        &self,
        session_uuid: Option<[u64; 2]>,
        agent_id: Option<u64>,
        min_ts: u64,
        max_ts: u64,
    ) -> Vec<(AiTraceRecord, S3ACoordinate)>;

    fn query_academic_papers_3d(
        &self,
        min_topic: [f32; 3],
        max_topic: [f32; 3],
        min_year: u16,
        max_year: u16,
    ) -> Vec<(AcademicPaperRecord, S3ACoordinate)>;

    fn query_research_graph(
        &self,
        subject_hash: Option<u64>,
        predicate_id: Option<u32>,
    ) -> Vec<(ResearchGraphEdgeRecord, S3ACoordinate)>;

    fn query_srs_workspace(
        &self,
        project_uuid: Option<[u64; 2]>,
    ) -> Vec<(SrsWorkspaceRecord, S3ACoordinate)>;

    fn query_agent_registry(
        &self,
        agent_id: Option<u64>,
    ) -> Vec<(AgentRegistryRecord, S3ACoordinate)>;

    fn query_note_claims(
        &self,
        session_uuid: Option<[u64; 2]>,
        query_text: Option<&str>,
    ) -> Vec<(ScholarNoteClaimRecord, S3ACoordinate)>;

    fn query_citation_fingerprints(
        &self,
        paper_id: Option<u64>,
    ) -> Vec<(CitationFingerprintRecord, S3ACoordinate)>;

    fn query_srs_manifest(&self) -> Option<SrsManifestHeader>;
}

impl<'a> ScholarQueryExt for QuerySieve<'a> {
    fn query_human_lrs(
        &self,
        session_uuid: Option<[u64; 2]>,
        actor_hash: Option<u64>,
        min_ts: u64,
        max_ts: u64,
    ) -> Vec<(HumanLrsRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::HUMAN_LRS {
                continue;
            }

            if can_reject_tile_time(header, min_ts, max_ts) {
                continue;
            }

            if let Some(session) = session_uuid {
                if can_reject_tile_bloom(header, session[0] ^ session[1]) {
                    continue;
                }
            }

            let records: &[HumanLrsRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if rec.timestamp_sec >= min_ts && rec.timestamp_sec <= max_ts {
                    if let Some(s) = session_uuid {
                        if rec.session_uuid != s {
                            continue;
                        }
                    }
                    if let Some(a) = actor_hash {
                        if rec.actor_hash != a {
                            continue;
                        }
                    }
                    if !rec.is_tombstone() {
                        let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                        results.push((*rec, coord));
                    }
                }
            }
        }

        results
    }

    fn query_ai_traces(
        &self,
        session_uuid: Option<[u64; 2]>,
        agent_id: Option<u64>,
        min_ts: u64,
        max_ts: u64,
    ) -> Vec<(AiTraceRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::AI_TRACE_LOG {
                continue;
            }

            if can_reject_tile_time(header, min_ts, max_ts) {
                continue;
            }

            if let Some(session) = session_uuid {
                if can_reject_tile_bloom(header, session[0] ^ session[1]) {
                    continue;
                }
            }

            let records: &[AiTraceRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if rec.timestamp_sec >= min_ts && rec.timestamp_sec <= max_ts {
                    if let Some(s) = session_uuid {
                        if rec.session_uuid != s {
                            continue;
                        }
                    }
                    if let Some(a) = agent_id {
                        if rec.agent_id != a {
                            continue;
                        }
                    }
                    if !rec.is_tombstone() {
                        let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                        results.push((*rec, coord));
                    }
                }
            }
        }

        results
    }

    fn query_academic_papers_3d(
        &self,
        min_topic: [f32; 3],
        max_topic: [f32; 3],
        min_year: u16,
        max_year: u16,
    ) -> Vec<(AcademicPaperRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::ACADEMIC_PAPERS {
                continue;
            }

            let records: &[AcademicPaperRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if rec.year >= min_year
                    && rec.year <= max_year
                    && rec.topic_x >= min_topic[0]
                    && rec.topic_x <= max_topic[0]
                    && rec.topic_y >= min_topic[1]
                    && rec.topic_y <= max_topic[1]
                    && rec.topic_z >= min_topic[2]
                    && rec.topic_z <= max_topic[2]
                {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    results.push((*rec, coord));
                }
            }
        }

        results
    }

    fn query_research_graph(
        &self,
        subject_hash: Option<u64>,
        predicate_id: Option<u32>,
    ) -> Vec<(ResearchGraphEdgeRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::RESEARCH_GRAPH {
                continue;
            }

            if let Some(sub) = subject_hash {
                if can_reject_tile_bloom(header, sub) {
                    continue;
                }
            }

            let records: &[ResearchGraphEdgeRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if let Some(s) = subject_hash {
                    if rec.subject_hash != s {
                        continue;
                    }
                }
                if let Some(p) = predicate_id {
                    if rec.predicate_id != p {
                        continue;
                    }
                }
                let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                results.push((*rec, coord));
            }
        }

        results
    }

    fn query_srs_workspace(
        &self,
        project_uuid: Option<[u64; 2]>,
    ) -> Vec<(SrsWorkspaceRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::SRS_WORKSPACE {
                continue;
            }

            if let Some(proj) = project_uuid {
                if can_reject_tile_bloom(header, proj[0] ^ proj[1]) {
                    continue;
                }
            }

            let records: &[SrsWorkspaceRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if let Some(p) = project_uuid {
                    if rec.project_uuid != p {
                        continue;
                    }
                }
                let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                results.push((*rec, coord));
            }
        }

        results
    }

    fn query_agent_registry(
        &self,
        agent_id: Option<u64>,
    ) -> Vec<(AgentRegistryRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::AGENT_REGISTRY {
                continue;
            }

            if let Some(id) = agent_id {
                if can_reject_tile_bloom(header, id) {
                    continue;
                }
            }

            let records: &[AgentRegistryRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if let Some(id) = agent_id {
                    if rec.agent_id != id {
                        continue;
                    }
                }
                if rec.is_active() {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    results.push((*rec, coord));
                }
            }
        }

        results
    }

    fn query_note_claims(
        &self,
        session_uuid: Option<[u64; 2]>,
        query_text: Option<&str>,
    ) -> Vec<(ScholarNoteClaimRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::NOTE_CLAIMS {
                continue;
            }

            if let Some(session) = session_uuid {
                if can_reject_tile_bloom(header, session[0] ^ session[1]) {
                    continue;
                }
            }

            let records: &[ScholarNoteClaimRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if let Some(s) = session_uuid {
                    if rec.session_uuid != s {
                        continue;
                    }
                }
                if let Some(text) = query_text {
                    if !rec.matches_query(text) {
                        continue;
                    }
                }
                if !rec.is_tombstone() {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    results.push((*rec, coord));
                }
            }
        }

        results
    }

    fn query_srs_manifest(&self) -> Option<SrsManifestHeader> {
        let tile_count = self.reader().tile_count() as usize;
        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type == ScholarTileType::SRS_MANIFEST {
                let records: &[SrsManifestHeader] = bytemuck::cast_slice(payload);
                if let Some(manifest) = records.first() {
                    return Some(*manifest);
                }
            }
        }
        None
    }

    fn query_citation_fingerprints(
        &self,
        paper_id: Option<u64>,
    ) -> Vec<(CitationFingerprintRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader().tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader().get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != ScholarTileType::CITATION_FINGERPRINT {
                continue;
            }

            if let Some(pid) = paper_id {
                if can_reject_tile_bloom(header, pid) {
                    continue;
                }
            }

            let records: &[CitationFingerprintRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden()
                    && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32)
                {
                    continue;
                }
                if let Some(pid) = paper_id {
                    if rec.paper_id != pid {
                        continue;
                    }
                }
                if rec.is_active() {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    results.push((*rec, coord));
                }
            }
        }

        results
    }
}
