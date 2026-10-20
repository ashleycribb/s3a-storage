//! Interprets arbitrary bytes as a 192-byte DelegationGrantRecord and verifies it.
#![no_main]
use libfuzzer_sys::fuzz_target;
use s3a_vault::records::DelegationGrantRecord;

fuzz_target!(|data: &[u8]| {
    let n = core::mem::size_of::<DelegationGrantRecord>();
    if data.len() < n {
        return;
    }
    let g: DelegationGrantRecord = bytemuck::pod_read_unaligned(&data[..n]);
    let _ = s3a_vault::delegation::verify_delegation_grant(&g);
});
