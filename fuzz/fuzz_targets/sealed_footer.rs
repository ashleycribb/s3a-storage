//! Reads the v2 sealed end-cap / index footer from arbitrary bytes (tail-only reader).
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut cur = std::io::Cursor::new(data);
    if let Ok((_cap, hdr, _entries)) = s3a_engine::read_sealed_index_v2(&mut cur, data.len() as u64) {
        let _ = s3a_engine::verify_seal_chain(&[hdr]);
    }
});
