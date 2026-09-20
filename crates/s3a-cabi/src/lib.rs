use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::ptr;

use s3a_core::SrsManifestHeader;
use s3a_engine::{Compactor, MmapReader, QuerySieve, S3ACrudEngine, TileFusionEngine, TelemetryRecord, S3ACoordinate};

#[repr(C)]
pub struct S3AHandle {
    _private: [u8; 0],
}

/// Opens an existing S3A archive using memory mapping.
#[no_mangle]
pub unsafe extern "C" fn s3a_open(path: *const c_char) -> *mut S3AHandle {
    if path.is_null() {
        return ptr::null_mut();
    }
    let c_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    match MmapReader::open(c_str) {
        Ok(reader) => Box::into_raw(Box::new(reader)) as *mut S3AHandle,
        Err(_) => ptr::null_mut(),
    }
}

/// Closes an open S3A handle and releases memory mapping.
#[no_mangle]
pub unsafe extern "C" fn s3a_close(handle: *mut S3AHandle) {
    if !handle.is_null() {
        let _ = Box::from_raw(handle as *mut MmapReader);
    }
}

/// Returns the total tile count in the archive.
#[no_mangle]
pub unsafe extern "C" fn s3a_tile_count(handle: *const S3AHandle) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let reader = &*(handle as *const MmapReader);
    reader.tile_count()
}

/// Queries telemetry records within a time range.
#[no_mangle]
pub unsafe extern "C" fn s3a_query_telemetry(
    handle: *const S3AHandle,
    min_ts: u64,
    max_ts: u64,
    out_buffer: *mut TelemetryRecord,
    max_out_len: usize,
    out_len: *mut usize,
) -> c_int {
    if handle.is_null() || out_len.is_null() {
        return -1;
    }

    let reader = &*(handle as *const MmapReader);
    let sieve = QuerySieve::new(reader);
    let results = sieve.query_telemetry(min_ts, max_ts, None, None);

    let count = results.len().min(max_out_len);
    if !out_buffer.is_null() && count > 0 {
        ptr::copy_nonoverlapping(results.as_ptr(), out_buffer, count);
    }

    *out_len = results.len();
    0
}

/// Performs instantaneous O(1) zero-copy lookup by S3ACoordinate.
#[no_mangle]
pub unsafe extern "C" fn s3a_lookup_coordinate(
    path: *const c_char,
    level: u16,
    tile_id: u32,
    record_offset: u32,
    out_record: *mut TelemetryRecord,
) -> c_int {
    if path.is_null() || out_record.is_null() {
        return -1;
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let coord = S3ACoordinate::new(level, tile_id, record_offset);
    match S3ACrudEngine::open_or_create(path_str) {
        Ok(engine) => match engine.read_by_coordinate::<TelemetryRecord>(&coord) {
            Ok(rec) => {
                *out_record = rec;
                0
            }
            Err(_) => -1,
        },
        Err(_) => -1,
    }
}

/// Inserts telemetry records into an S3A archive (CREATE).
#[no_mangle]
pub unsafe extern "C" fn s3a_insert_telemetry(
    path: *const c_char,
    records: *const TelemetryRecord,
    num_records: usize,
) -> c_int {
    if path.is_null() || records.is_null() || num_records == 0 {
        return -1;
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let slice = std::slice::from_raw_parts(records, num_records);
    match S3ACrudEngine::open_or_create(path_str) {
        Ok(engine) => match engine.create_telemetry(slice) {
            Ok(_) => 0,
            Err(_) => -1,
        },
        Err(_) => -1,
    }
}

/// Soft-deletes a telemetry record by appending a tombstone marker (DELETE).
#[no_mangle]
pub unsafe extern "C" fn s3a_delete_telemetry(
    path: *const c_char,
    sensor_id: u32,
    metric_id: u32,
    timestamp: u64,
) -> c_int {
    if path.is_null() {
        return -1;
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    match S3ACrudEngine::open_or_create(path_str) {
        Ok(engine) => match engine.delete_telemetry(sensor_id, metric_id, timestamp) {
            Ok(true) => 0,
            Ok(false) => 1, // Record not found
            Err(_) => -1,
        },
        Err(_) => -1,
    }
}

/// Compacts multiple S3A input archives into a single optimized S3A output archive.
#[no_mangle]
pub unsafe extern "C" fn s3a_compact(
    input_paths: *const *const c_char,
    num_inputs: usize,
    output_path: *const c_char,
) -> c_int {
    if input_paths.is_null() || output_path.is_null() || num_inputs == 0 {
        return -1;
    }

    let out_str = match CStr::from_ptr(output_path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let mut in_strs = Vec::with_capacity(num_inputs);
    for i in 0..num_inputs {
        let p = *input_paths.add(i);
        if p.is_null() {
            return -1;
        }
        match CStr::from_ptr(p).to_str() {
            Ok(s) => in_strs.push(s),
            Err(_) => return -1,
        }
    }

    match Compactor::compact(&in_strs, &out_str) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

/// Fuses multiple telemetry Hyper-Tiles across distinct archives into a single consolidated Hyper-Tile archive.
#[no_mangle]
pub unsafe extern "C" fn s3a_fuse_telemetry(
    input_paths: *const *const c_char,
    num_inputs: usize,
    output_path: *const c_char,
) -> c_int {
    if input_paths.is_null() || output_path.is_null() || num_inputs == 0 {
        return -1;
    }

    let out_str = match CStr::from_ptr(output_path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let mut in_strs = Vec::with_capacity(num_inputs);
    for i in 0..num_inputs {
        let p = *input_paths.add(i);
        if p.is_null() {
            return -1;
        }
        match CStr::from_ptr(p).to_str() {
            Ok(s) => in_strs.push(s),
            Err(_) => return -1,
        }
    }

    match TileFusionEngine::fuse_telemetry_tiles(&in_strs, &out_str) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

/// Exports live S3A academic project database into a standalone, portable .snapshot.s3a container.
#[no_mangle]
pub unsafe extern "C" fn s3a_export_srs(
    live_path: *const c_char,
    snapshot_path: *const c_char,
    project_uuid_high: u64,
    project_uuid_low: u64,
    out_total_records: *mut u64,
) -> c_int {
    if live_path.is_null() || snapshot_path.is_null() {
        return -1;
    }

    let live_str = match CStr::from_ptr(live_path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let snap_str = match CStr::from_ptr(snapshot_path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let p_uuid = [project_uuid_high, project_uuid_low];
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let manifest = SrsManifestHeader::new(p_uuid, [0x1111, ts], [0, 0], [0, 0], ts, 0, 0);

    match S3ACrudEngine::open_or_create(live_str) {
        Ok(engine) => match engine.export_srs_snapshot(snap_str, &manifest, None, Some(p_uuid)) {
            Ok(bundle) => {
                if !out_total_records.is_null() {
                    *out_total_records = bundle.manifest.total_records;
                }
                0
            }
            Err(_) => -1,
        },
        Err(_) => -1,
    }
}

/// Ingests a portable .snapshot.s3a container via zero-copy mmap, verifying hardware CRC32C.
#[no_mangle]
pub unsafe extern "C" fn s3a_intake_srs(
    snapshot_path: *const c_char,
    out_papers_count: *mut usize,
    out_edges_count: *mut usize,
    out_human_count: *mut usize,
    out_ai_count: *mut usize,
) -> c_int {
    if snapshot_path.is_null() {
        return -1;
    }

    let snap_str = match CStr::from_ptr(snapshot_path).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    match S3ACrudEngine::intake_srs_snapshot(snap_str) {
        Ok(bundle) => {
            if !out_papers_count.is_null() { *out_papers_count = bundle.papers.len(); }
            if !out_edges_count.is_null() { *out_edges_count = bundle.evidence_edges.len(); }
            if !out_human_count.is_null() { *out_human_count = bundle.human_decisions.len(); }
            if !out_ai_count.is_null() { *out_ai_count = bundle.ai_traces.len(); }
            0
        }
        Err(_) => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use s3a_engine::{TileType, TileWriter};

    struct TestTempFile {
        path: std::path::PathBuf,
    }

    impl TestTempFile {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut path = std::env::temp_dir();
            path.push(format!("s3a_cabi_test_{}_{}_{}.s3a", std::process::id(), ts, id));
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TestTempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn test_cabi_open_query_compact_and_fuse() {
        let temp_file1 = TestTempFile::new();
        let temp_file2 = TestTempFile::new();
        let temp_out = TestTempFile::new();


        let path_str1 = temp_file1.path().to_str().unwrap();
        let path_str2 = temp_file2.path().to_str().unwrap();
        let out_str = temp_out.path().to_str().unwrap();

        let mut writer1 = TileWriter::create(temp_file1.path()).unwrap();
        writer1.write_hyper_tile(TileType::TELEMETRY, &[TelemetryRecord::new(500, 1, 10, 99.9)], Some(&[500]), None).unwrap();

        let mut writer2 = TileWriter::create(temp_file2.path()).unwrap();
        writer2.write_hyper_tile(TileType::TELEMETRY, &[TelemetryRecord::new(600, 1, 10, 101.2)], Some(&[600]), None).unwrap();

        let c_p1 = CString::new(path_str1).unwrap();
        let c_p2 = CString::new(path_str2).unwrap();
        let c_out = CString::new(out_str).unwrap();

        let inputs = vec![c_p1.as_ptr(), c_p2.as_ptr()];

        unsafe {
            let res_fuse = s3a_fuse_telemetry(inputs.as_ptr(), 2, c_out.as_ptr());
            assert_eq!(res_fuse, 0);

            let handle = s3a_open(c_out.as_ptr());
            assert!(!handle.is_null());

            let count = s3a_tile_count(handle);
            assert_eq!(count, 1);

            let mut out_records = [TelemetryRecord::new(0, 0, 0, 0.0); 10];
            let mut out_len = 0;
            let res = s3a_query_telemetry(handle, 0, 1000, out_records.as_mut_ptr(), 10, &mut out_len);
            assert_eq!(res, 0);
            assert_eq!(out_len, 2);

            s3a_close(handle);
        }
    }

    #[test]
    fn test_cabi_crud_insert_delete_lookup() {
        let temp_file = TestTempFile::new();
        let path_str = temp_file.path().to_str().unwrap();
        let c_path = CString::new(path_str).unwrap();

        let rec = TelemetryRecord::new(1000, 5, 2, 88.5);

        unsafe {
            let res_insert = s3a_insert_telemetry(c_path.as_ptr(), &rec, 1);
            assert_eq!(res_insert, 0);

            let mut fetched_rec = TelemetryRecord::new(0, 0, 0, 0.0);
            let res_lookup = s3a_lookup_coordinate(c_path.as_ptr(), 0, 0, 0, &mut fetched_rec);
            assert_eq!(res_lookup, 0);
            assert_eq!(fetched_rec.timestamp, 1000);
            assert_eq!(fetched_rec.value, 88.5);

            let res_del = s3a_delete_telemetry(c_path.as_ptr(), 5, 2, 1000);
            assert_eq!(res_del, 0);
        }
    }

    #[test]
    fn test_cabi_srs_snapshot_export_and_intake() {
        use s3a_engine::AcademicPaperRecord;

        let live_db = TestTempFile::new();
        let snap_file = TestTempFile::new();

        let live_path = CString::new(live_db.path().to_str().unwrap()).unwrap();
        let snap_path = CString::new(snap_file.path().to_str().unwrap()).unwrap();

        // Seed live DB with a paper
        let engine = S3ACrudEngine::open_or_create(live_db.path()).unwrap();
        let paper = AcademicPaperRecord::new(5555, 1600000000, 100, 0.1, 0.2, 0.3, 10, 2024, 1, 1, 0, 99);
        engine.create_academic_papers(&[paper]).unwrap();

        unsafe {
            let mut total_records = 0u64;
            let export_res = s3a_export_srs(live_path.as_ptr(), snap_path.as_ptr(), 0x1122, 0x3344, &mut total_records);
            assert_eq!(export_res, 0);
            assert_eq!(total_records, 1);

            let mut papers = 0usize;
            let mut edges = 0usize;
            let mut human = 0usize;
            let mut ai = 0usize;

            let intake_res = s3a_intake_srs(snap_path.as_ptr(), &mut papers, &mut edges, &mut human, &mut ai);
            assert_eq!(intake_res, 0);
            assert_eq!(papers, 1);
            assert_eq!(edges, 0);
            assert_eq!(human, 0);
            assert_eq!(ai, 0);
        }
    }
}

