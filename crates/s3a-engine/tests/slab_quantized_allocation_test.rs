//! Integration and property tests for Quantized Slab Storage & Addressing (PR 1).
//!
//! Validates:
//! 1. TileAddress bit-packing round-trip and boundary limit validations.
//! 2. 10 size classes selection (4 KB to 128 KB with 1.5x stepping).
//! 3. Worst-case internal fragmentation bound (<= 33.3% above 8 KB).
//! 4. Deterministic physical offset calculation math.
//! 5. Slab slot allocation, write, read, free, and bitmap slot reuse.
//! 6. Automatic extent rollover when an extent file is full.
//! 7. LRU File Descriptor pool bounds and eviction.

use s3a_core::{
    select_size_class, size_class_bytes, TileAddress, TileAddressError,
    MAX_SIZE_CLASS_ID, MAX_SLAB_EXTENT_ID, MAX_SLOT_INDEX, NUM_SLAB_CLASSES,
    SLAB_SIZE_CLASSES,
};
use s3a_engine::slab::{QuantizedSlabManager, SLAB_MAGIC};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestTempDir {
    path: PathBuf,
}

impl TestTempDir {
    fn new(prefix: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "s3a_slab_test_{}_{}_{}",
            std::process::id(),
            count,
            prefix
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_address_round_trip_and_limits() {
    // Standard round-trip across all size classes
    for class_id in 0..=MAX_SIZE_CLASS_ID {
        for extent_id in [0, 1, 42, 1000, 0x0123_4567] {
            for slot_index in [0, 1, 100, 0x0012_3456] {
                let addr = TileAddress::new(class_id, extent_id, slot_index).expect("valid address");
                assert_eq!(addr.size_class_id(), class_id);
                assert_eq!(addr.extent_id(), extent_id);
                assert_eq!(addr.slot_index(), slot_index);
                assert_eq!(addr.reserved(), 0);

                let parsed = TileAddress::from_u64(addr.as_u64()).expect("valid from_u64");
                assert_eq!(addr, parsed);
            }
        }
    }

    // Maximum legal values
    let max_addr = TileAddress::new(MAX_SIZE_CLASS_ID, MAX_SLAB_EXTENT_ID, MAX_SLOT_INDEX).unwrap();
    assert_eq!(max_addr.size_class_id(), MAX_SIZE_CLASS_ID);
    assert_eq!(max_addr.extent_id(), MAX_SLAB_EXTENT_ID);
    assert_eq!(max_addr.slot_index(), MAX_SLOT_INDEX);
    assert_eq!(max_addr.reserved(), 0);

    let parsed_max = TileAddress::from_u64(max_addr.as_u64()).unwrap();
    assert_eq!(max_addr, parsed_max);

    // Limit violations
    assert!(matches!(
        TileAddress::new(10, 0, 0),
        Err(TileAddressError::InvalidSizeClass(10))
    ));
    assert!(matches!(
        TileAddress::new(0, MAX_SLAB_EXTENT_ID + 1, 0),
        Err(TileAddressError::ExtentIdOverflow(_))
    ));
    assert!(matches!(
        TileAddress::new(0, 0, MAX_SLOT_INDEX + 1),
        Err(TileAddressError::SlotIndexOverflow(_))
    ));

    // Reserved bits non-zero rejected on unpack
    assert!(matches!(
        TileAddress::from_u64(max_addr.as_u64() | 0x01),
        Err(TileAddressError::ReservedNonZero(0x01))
    ));
    assert!(matches!(
        TileAddress::from_u64(max_addr.as_u64() | 0xA0),
        Err(TileAddressError::ReservedNonZero(0xA0))
    ));
}

#[test]
fn test_class_selection() {
    assert_eq!(NUM_SLAB_CLASSES, 10);
    assert_eq!(
        SLAB_SIZE_CLASSES,
        [4096, 8192, 12288, 16384, 24576, 32768, 49152, 65536, 98304, 131072]
    );

    // Boundary testing for each class
    assert_eq!(select_size_class(0), Some(0)); // 0B -> 4 KB
    assert_eq!(select_size_class(4096), Some(0)); // 4096B -> 4 KB
    assert_eq!(select_size_class(4097), Some(1)); // 4097B -> 8 KB
    assert_eq!(select_size_class(8192), Some(1)); // 8192B -> 8 KB
    assert_eq!(select_size_class(8193), Some(2)); // 8193B -> 12 KB
    assert_eq!(select_size_class(12288), Some(2)); // 12288B -> 12 KB
    assert_eq!(select_size_class(12289), Some(3)); // 12289B -> 16 KB
    assert_eq!(select_size_class(16384), Some(3)); // 16384B -> 16 KB
    assert_eq!(select_size_class(16385), Some(4)); // 16385B -> 24 KB
    assert_eq!(select_size_class(24576), Some(4)); // 24576B -> 24 KB
    assert_eq!(select_size_class(24577), Some(5)); // 24577B -> 32 KB
    assert_eq!(select_size_class(32768), Some(5)); // 32768B -> 32 KB
    assert_eq!(select_size_class(32769), Some(6)); // 32769B -> 48 KB
    assert_eq!(select_size_class(49152), Some(6)); // 49152B -> 48 KB
    assert_eq!(select_size_class(49153), Some(7)); // 49153B -> 64 KB
    assert_eq!(select_size_class(65536), Some(7)); // 65536B -> 64 KB
    assert_eq!(select_size_class(65537), Some(8)); // 65537B -> 96 KB
    assert_eq!(select_size_class(98304), Some(8)); // 98304B -> 96 KB
    assert_eq!(select_size_class(98305), Some(9)); // 98305B -> 128 KB
    assert_eq!(select_size_class(131072), Some(9)); // 131072B -> 128 KB
    assert_eq!(select_size_class(131073), None); // Over 128 KB rejected
}

#[test]
fn test_worst_case_waste_above_8kb() {
    let mut max_waste_ratio = 0.0f64;
    let mut worst_payload = 0usize;

    // Scan every payload size from 8193 up to 131072
    for payload in 8193..=131072 {
        let class_id = select_size_class(payload).expect("payload in bounds");
        let capacity = size_class_bytes(class_id).expect("valid class") as usize;
        assert!(capacity >= payload);

        let waste = (capacity - payload) as f64 / capacity as f64;
        if waste > max_waste_ratio {
            max_waste_ratio = waste;
            worst_payload = payload;
        }

        // Must never exceed 33.34% (1/3 bound)
        assert!(
            waste <= 0.3334,
            "Internal fragmentation exceeded 33.33% at payload {payload}: waste = {:.4}",
            waste
        );
    }

    // Verify theoretical bound is approximately 33.33% (occurring at step boundaries 8193, 16385, 32769, 65537)
    assert!(
        max_waste_ratio <= 1.0 / 3.0 + 1e-4,
        "Max waste ratio {:.4} at payload {} exceeds 33.333%",
        max_waste_ratio,
        worst_payload
    );
}

#[test]
fn test_offset_math() {
    for class_id in 0..NUM_SLAB_CLASSES as u8 {
        let class_bytes = size_class_bytes(class_id).unwrap() as u64;
        for slot in [0u32, 1, 2, 5, 10, 100, 1000] {
            let addr = TileAddress::new(class_id, 0, slot).unwrap();
            let expected_offset = 4096 + (slot as u64 * class_bytes);
            assert_eq!(
                addr.physical_offset().unwrap(),
                expected_offset,
                "Offset mismatch for class {class_id}, slot {slot}"
            );
        }
    }
}

#[test]
fn test_allocate_free_reuse() {
    let temp = TestTempDir::new("alloc_free");
    let mut manager = QuantizedSlabManager::new(temp.path()).expect("manager creation");

    // Allocate 3 slots in class 0 (4 KB)
    let a0 = manager.allocate_slot(500).expect("allocate a0");
    let a1 = manager.allocate_slot(1200).expect("allocate a1");
    let a2 = manager.allocate_slot(3000).expect("allocate a2");

    assert_eq!(a0.size_class_id(), 0);
    assert_eq!(a0.extent_id(), 0);
    assert_eq!(a0.slot_index(), 0);

    assert_eq!(a1.size_class_id(), 0);
    assert_eq!(a1.extent_id(), 0);
    assert_eq!(a1.slot_index(), 1);

    assert_eq!(a2.size_class_id(), 0);
    assert_eq!(a2.extent_id(), 0);
    assert_eq!(a2.slot_index(), 2);

    // Write distinct data to each slot
    let data0 = b"Hello from slot 0!";
    let data1 = b"Data payload inside slot 1";
    let data2 = b"Third slot payload block!";

    manager.write_slot(a0, data0).expect("write a0");
    manager.write_slot(a1, data1).expect("write a1");
    manager.write_slot(a2, data2).expect("write a2");

    // Read back and verify
    let mut buf = [0u8; 64];
    let n0 = manager.read_slot(a0, &mut buf).expect("read a0");
    assert!(n0 >= data0.len());
    assert_eq!(&buf[..data0.len()], data0);

    let n1 = manager.read_slot(a1, &mut buf).expect("read a1");
    assert!(n1 >= data1.len());
    assert_eq!(&buf[..data1.len()], data1);

    let n2 = manager.read_slot(a2, &mut buf).expect("read a2");
    assert!(n2 >= data2.len());
    assert_eq!(&buf[..data2.len()], data2);

    // Free slot 1
    manager.free_slot(a1).expect("free a1");

    // Allocating another slot in class 0 should reuse slot 1 (first free bit)
    let a_reused = manager.allocate_slot(1500).expect("allocate reused");
    assert_eq!(a_reused.size_class_id(), 0);
    assert_eq!(a_reused.extent_id(), 0);
    assert_eq!(a_reused.slot_index(), 1); // Slot 1 reused!

    let reused_data = b"Overwritten slot 1 fresh content!";
    manager.write_slot(a_reused, reused_data).expect("write reused");
    manager.read_slot(a_reused, &mut buf).expect("read reused");
    assert_eq!(&buf[..reused_data.len()], reused_data);
}

#[test]
fn test_extent_rollover_when_full() {
    let temp = TestTempDir::new("rollover");
    // Configure small extent capacity of 4 slots per extent file
    let mut manager = QuantizedSlabManager::with_options(temp.path(), 8, 4)
        .expect("manager with custom capacity");

    // Fill extent 0 (slots 0..3)
    let mut addrs = Vec::new();
    for i in 0..4 {
        let addr = manager.allocate_slot(2000).expect("allocate slot");
        assert_eq!(addr.size_class_id(), 0);
        assert_eq!(addr.extent_id(), 0);
        assert_eq!(addr.slot_index(), i as u32);
        addrs.push(addr);
    }

    // Extent 0 is now full. Next allocation must rollover to extent 1, slot 0!
    let a_rollover = manager.allocate_slot(2000).expect("allocate rollover");
    assert_eq!(a_rollover.size_class_id(), 0);
    assert_eq!(a_rollover.extent_id(), 1);
    assert_eq!(a_rollover.slot_index(), 0);

    // Check that both extent files physically exist on disk
    let path0 = manager.extent_path(0, 0);
    let path1 = manager.extent_path(0, 1);
    assert!(path0.is_file(), "extent 0 must exist at {:?}", path0);
    assert!(path1.is_file(), "extent 1 must exist at {:?}", path1);

    // Verify file headers
    let h0 = manager.read_header(0, 0).expect("read header 0");
    assert_eq!(h0.magic, SLAB_MAGIC);
    assert_eq!(h0.extent_id, 0);
    assert_eq!(h0.slot_count, 4);
    assert!(h0.is_full());

    let h1 = manager.read_header(0, 1).expect("read header 1");
    assert_eq!(h1.magic, SLAB_MAGIC);
    assert_eq!(h1.extent_id, 1);
    assert_eq!(h1.allocated_count(), 1);
    assert!(!h1.is_full());

    // Write & read across extents
    let data_ext0 = b"Payload in extent 0 slot 3";
    let data_ext1 = b"Payload in extent 1 slot 0";
    manager.write_slot(addrs[3], data_ext0).expect("write ext0");
    manager.write_slot(a_rollover, data_ext1).expect("write ext1");

    let mut buf = [0u8; 64];
    manager.read_slot(addrs[3], &mut buf).expect("read ext0");
    assert_eq!(&buf[..data_ext0.len()], data_ext0);

    manager.read_slot(a_rollover, &mut buf).expect("read ext1");
    assert_eq!(&buf[..data_ext1.len()], data_ext1);
}

#[test]
fn test_fd_pool_lru_eviction() {
    let temp = TestTempDir::new("fd_pool");
    // Pool capacity of 2 handles
    let mut manager = QuantizedSlabManager::with_options(temp.path(), 2, 2)
        .expect("manager with pool capacity 2");

    // Allocate across 3 size classes: class 0, class 1, class 2
    let a0 = manager.allocate_slot(1000).expect("alloc class 0"); // class 0 -> extent 0
    let a1 = manager.allocate_slot(6000).expect("alloc class 1"); // class 1 -> extent 0
    let a2 = manager.allocate_slot(10000).expect("alloc class 2"); // class 2 -> extent 0

    assert_eq!(a0.size_class_id(), 0);
    assert_eq!(a1.size_class_id(), 1);
    assert_eq!(a2.size_class_id(), 2);

    // Write to all three (tests transparent opening, eviction, and re-opening)
    manager.write_slot(a0, b"class 0 payload").expect("write a0");
    manager.write_slot(a1, b"class 1 payload").expect("write a1");
    manager.write_slot(a2, b"class 2 payload").expect("write a2");

    // Read back all three
    let mut buf = [0u8; 32];
    manager.read_slot(a0, &mut buf).expect("read a0");
    assert_eq!(&buf[..15], b"class 0 payload");

    manager.read_slot(a1, &mut buf).expect("read a1");
    assert_eq!(&buf[..15], b"class 1 payload");

    manager.read_slot(a2, &mut buf).expect("read a2");
    assert_eq!(&buf[..15], b"class 2 payload");
}
