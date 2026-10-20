//! Loads arbitrary bytes into simulated NOR flash and runs the edge recovery reader.
#![no_main]
use libfuzzer_sys::fuzz_target;
use s3a_edge::{writer::EdgeWriter, RamFlash};

const CAP: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    let mut flash: Box<RamFlash<CAP>> = Box::new(RamFlash::new(256, 4096));
    let n = data.len().min(CAP);
    flash.memory[..n].copy_from_slice(&data[..n]);
    let _ = EdgeWriter::<_, 4096>::recover(*flash, [0u8; 16]);
});
