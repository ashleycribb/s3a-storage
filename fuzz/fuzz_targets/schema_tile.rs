//! B1: decodes arbitrary bytes as a Schema tile payload at any alignment.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = s3a_core::schema::decode_schema_payload(data);
    if data.len() > 1 {
        let _ = s3a_core::schema::decode_schema_payload(&data[1..]);
    }
});
