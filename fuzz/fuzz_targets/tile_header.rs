//! Opens arbitrary bytes as an .s3a file: header slots, tile headers, checksums.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Write;

fuzz_target!(|data: &[u8]| {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.s3a");
    std::fs::File::create(&path).unwrap().write_all(data).unwrap();
    if let Ok(r) = s3a_engine::MmapReader::open(&path) {
        let _ = r.verify_checksums();
    }
    let _ = s3a_engine::recover_archive(&path);
});
