//! Comprehensive tests for N-Dimensional Array Tile (Tile 66 / 0x0042)
//!
//! Validates:
//! 1. 2D, 3D and 4D round-trip equality
//! 2. Slice across chunk boundaries
//! 3. Missing chunk = fill value
//! 4. Dtype coverage (i8, u8, i16, u16, i32, u32, f32, f64)
//! 5. Wrong-shape write rejected

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::array::{ArrayDataType, ArrayTileBuilder, ArrayTileView};
use s3a_engine::{MmapReader, S3ACrudEngine};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestTempFile {
    path: PathBuf,
}

impl TestTempFile {
    fn new(suffix: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "s3a_array_test_{}_{}_{}.s3a",
            std::process::id(),
            count,
            suffix
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn test_array_roundtrip_2d_3d_4d() {
    // 2D Array: 4 x 5 float32 matrix
    let shape_2d = [4u64, 5u64];
    let coords_2d = [0u64, 0u64];
    let mut builder_2d =
        ArrayTileBuilder::new(2, ArrayDataType::Float32, &shape_2d, &coords_2d).unwrap();
    builder_2d.set_axis_names("rows,cols");

    let data_2d: Vec<f32> = (0..20).map(|x| x as f32 * 0.5).collect();
    builder_2d.set_data(&data_2d).unwrap();
    let payload_2d = builder_2d.build();

    let view_2d = ArrayTileView::parse(&payload_2d).expect("parse 2d must succeed");
    assert_eq!(view_2d.header.ndim, 2);
    assert_eq!(view_2d.header.axis_names_as_str(), "rows,cols");
    assert!(view_2d.verify_chunk_hash());
    let elems_2d: &[f32] = view_2d.read_elements().unwrap();
    assert_eq!(elems_2d, data_2d.as_slice());
    assert_eq!(
        view_2d.get_element::<f32>(&[2, 3]),
        Some(data_2d[2 * 5 + 3])
    );

    // 3D Array: 3 x 4 x 2 int32 cube
    let shape_3d = [3u64, 4u64, 2u64];
    let coords_3d = [0u64, 0u64, 0u64];
    let mut builder_3d =
        ArrayTileBuilder::new(3, ArrayDataType::Int32, &shape_3d, &coords_3d).unwrap();
    builder_3d.set_axis_names("x,y,z");

    let data_3d: Vec<i32> = (0..24).map(|x| (x * 10) as i32).collect();
    builder_3d.set_data(&data_3d).unwrap();
    let payload_3d = builder_3d.build();

    let view_3d = ArrayTileView::parse(&payload_3d).expect("parse 3d must succeed");
    assert_eq!(view_3d.header.ndim, 3);
    assert!(view_3d.verify_chunk_hash());
    let elems_3d: &[i32] = view_3d.read_elements().unwrap();
    assert_eq!(elems_3d, data_3d.as_slice());
    // (x=1, y=2, z=1) -> index = (1 * 4 + 2) * 2 + 1 = 13
    assert_eq!(view_3d.get_element::<i32>(&[1, 2, 1]), Some(data_3d[13]));

    // 4D Array: 2 x 3 x 2 x 2 float64 hyper-volume
    let shape_4d = [2u64, 3u64, 2u64, 2u64];
    let coords_4d = [0u64, 0u64, 0u64, 0u64];
    let mut builder_4d =
        ArrayTileBuilder::new(4, ArrayDataType::Float64, &shape_4d, &coords_4d).unwrap();
    builder_4d.set_axis_names("batch,channels,height,width");

    let data_4d: Vec<f64> = (0..24).map(|x| x as f64 * 3.14).collect();
    builder_4d.set_data(&data_4d).unwrap();
    let payload_4d = builder_4d.build();

    let view_4d = ArrayTileView::parse(&payload_4d).expect("parse 4d must succeed");
    assert_eq!(view_4d.header.ndim, 4);
    assert!(view_4d.verify_chunk_hash());
    let elems_4d: &[f64] = view_4d.read_elements().unwrap();
    assert_eq!(elems_4d, data_4d.as_slice());
}

#[test]
fn test_array_slice_across_chunk_boundaries() {
    let tmp = TestTempFile::new("slice_chunks");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    // Create a partitioned 2D array: 2 chunks of shape [4, 4]
    // Chunk 0 covers rows [0..4], cols [0..4] (coords [0, 0])
    // Chunk 1 covers rows [0..4], cols [4..8] (coords [0, 4])
    let shape = [4u64, 4u64];

    let mut chunk0 = ArrayTileBuilder::new(2, ArrayDataType::Int32, &shape, &[0, 0]).unwrap();
    let data0: Vec<i32> = (0..16).map(|i| 100 + i as i32).collect();
    chunk0.set_data(&data0).unwrap();
    let payload0 = chunk0.build();

    let mut chunk1 = ArrayTileBuilder::new(2, ArrayDataType::Int32, &shape, &[0, 4]).unwrap();
    let data1: Vec<i32> = (0..16).map(|i| 200 + i as i32).collect();
    chunk1.set_data(&data1).unwrap();
    let payload1 = chunk1.build();

    engine.write_array_chunk(&payload0).unwrap();
    engine.write_array_chunk(&payload1).unwrap();

    // Verify written tiles
    let reader = MmapReader::open(tmp.path()).unwrap();
    assert_eq!(reader.tile_count(), 2);
    drop(reader);

    // Query slice bridging across both chunks:
    // rows [1..3] (2 rows), cols [2..6] (4 columns)
    // Global columns 2..4 come from Chunk 0 (local cols 2..4)
    // Global columns 4..6 come from Chunk 1 (local cols 0..2)
    let slice = engine
        .read_array_slice::<i32>(&[(1, 3), (2, 6)], -1)
        .unwrap();
    assert_eq!(slice.len(), 2 * 4); // 8 elements

    // Row 1 (global row 1):
    // col 2: chunk0 row 1, col 2 -> 100 + (1 * 4 + 2) = 106
    // col 3: chunk0 row 1, col 3 -> 100 + (1 * 4 + 3) = 107
    // col 4: chunk1 row 1, col 0 -> 200 + (1 * 4 + 0) = 204
    // col 5: chunk1 row 1, col 1 -> 200 + (1 * 4 + 1) = 205
    assert_eq!(slice[0], 106);
    assert_eq!(slice[1], 107);
    assert_eq!(slice[2], 204);
    assert_eq!(slice[3], 205);

    // Row 2 (global row 2):
    // col 2: chunk0 row 2, col 2 -> 100 + (2 * 4 + 2) = 110
    // col 3: chunk0 row 2, col 3 -> 100 + (2 * 4 + 3) = 111
    // col 4: chunk1 row 2, col 0 -> 200 + (2 * 4 + 0) = 208
    // col 5: chunk1 row 2, col 1 -> 200 + (2 * 4 + 1) = 209
    assert_eq!(slice[4], 110);
    assert_eq!(slice[5], 111);
    assert_eq!(slice[6], 208);
    assert_eq!(slice[7], 209);
}

#[test]
fn test_missing_chunk_returns_fill_value() {
    let tmp = TestTempFile::new("fill_value");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    // Write only Chunk at coords [0, 0] with shape [2, 2]
    let mut chunk = ArrayTileBuilder::new(2, ArrayDataType::Float64, &[2, 2], &[0, 0]).unwrap();
    let data: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0];
    chunk.set_data(&data).unwrap();
    engine.write_array_chunk(&chunk.build()).unwrap();

    // Slice rows [0..2], cols [0..4] (2 rows x 4 cols = 8 elements)
    // Cols [0..2] exist, Cols [2..4] are missing chunk space -> MUST return fill_value (-999.0)
    let fill = -999.0f64;
    let slice = engine
        .read_array_slice::<f64>(&[(0, 2), (0, 4)], fill)
        .unwrap();
    assert_eq!(slice.len(), 8);

    // Row 0
    assert_eq!(slice[0], 1.0);
    assert_eq!(slice[1], 2.0);
    assert_eq!(slice[2], fill);
    assert_eq!(slice[3], fill);

    // Row 1
    assert_eq!(slice[4], 3.0);
    assert_eq!(slice[5], 4.0);
    assert_eq!(slice[6], fill);
    assert_eq!(slice[7], fill);
}

#[test]
fn test_dtype_coverage() {
    // i8
    let mut b_i8 = ArrayTileBuilder::new(1, ArrayDataType::Int8, &[4], &[0]).unwrap();
    b_i8.set_data(&[-10i8, 0, 10, 20]).unwrap();
    let p_i8 = b_i8.build();
    let v_i8 = ArrayTileView::parse(&p_i8).unwrap();
    assert_eq!(v_i8.read_elements::<i8>().unwrap(), &[-10, 0, 10, 20]);

    // u8
    let mut b_u8 = ArrayTileBuilder::new(1, ArrayDataType::UInt8, &[4], &[0]).unwrap();
    b_u8.set_data(&[0u8, 50, 100, 255]).unwrap();
    let p_u8 = b_u8.build();
    let v_u8 = ArrayTileView::parse(&p_u8).unwrap();
    assert_eq!(v_u8.read_elements::<u8>().unwrap(), &[0, 50, 100, 255]);

    // i16
    let mut b_i16 = ArrayTileBuilder::new(1, ArrayDataType::Int16, &[3], &[0]).unwrap();
    b_i16.set_data(&[-32000i16, 0, 32000]).unwrap();
    let p_i16 = b_i16.build();
    let v_i16 = ArrayTileView::parse(&p_i16).unwrap();
    assert_eq!(v_i16.read_elements::<i16>().unwrap(), &[-32000, 0, 32000]);

    // u16
    let mut b_u16 = ArrayTileBuilder::new(1, ArrayDataType::UInt16, &[3], &[0]).unwrap();
    b_u16.set_data(&[0u16, 1000, 65000]).unwrap();
    let p_u16 = b_u16.build();
    let v_u16 = ArrayTileView::parse(&p_u16).unwrap();
    assert_eq!(v_u16.read_elements::<u16>().unwrap(), &[0, 1000, 65000]);

    // i32
    let mut b_i32 = ArrayTileBuilder::new(1, ArrayDataType::Int32, &[2], &[0]).unwrap();
    b_i32.set_data(&[-100_000i32, 100_000]).unwrap();
    let p_i32 = b_i32.build();
    let v_i32 = ArrayTileView::parse(&p_i32).unwrap();
    assert_eq!(v_i32.read_elements::<i32>().unwrap(), &[-100_000, 100_000]);

    // u32
    let mut b_u32 = ArrayTileBuilder::new(1, ArrayDataType::UInt32, &[2], &[0]).unwrap();
    b_u32.set_data(&[10u32, 200_000]).unwrap();
    let p_u32 = b_u32.build();
    let v_u32 = ArrayTileView::parse(&p_u32).unwrap();
    assert_eq!(v_u32.read_elements::<u32>().unwrap(), &[10, 200_000]);

    // f32
    let mut b_f32 = ArrayTileBuilder::new(1, ArrayDataType::Float32, &[2], &[0]).unwrap();
    b_f32.set_data(&[1.25f32, -3.75]).unwrap();
    let p_f32 = b_f32.build();
    let v_f32 = ArrayTileView::parse(&p_f32).unwrap();
    assert_eq!(v_f32.read_elements::<f32>().unwrap(), &[1.25, -3.75]);

    // f64
    let mut b_f64 = ArrayTileBuilder::new(1, ArrayDataType::Float64, &[2], &[0]).unwrap();
    b_f64
        .set_data(&[core::f64::consts::PI, core::f64::consts::E])
        .unwrap();
    let p_f64 = b_f64.build();
    let v_f64 = ArrayTileView::parse(&p_f64).unwrap();
    assert_eq!(
        v_f64.read_elements::<f64>().unwrap(),
        &[core::f64::consts::PI, core::f64::consts::E]
    );
}

#[test]
fn test_wrong_shape_write_rejected() {
    // 1. Invalid ndim (0 or > 8)
    assert!(ArrayTileBuilder::new(0, ArrayDataType::Int32, &[], &[]).is_err());
    assert!(ArrayTileBuilder::new(9, ArrayDataType::Int32, &[1; 9], &[0; 9]).is_err());

    // 2. Zero in shape dimension
    assert!(ArrayTileBuilder::new(2, ArrayDataType::Int32, &[4, 0], &[0, 0]).is_err());

    // 3. Data size mismatch vs declared shape
    let mut builder = ArrayTileBuilder::new(2, ArrayDataType::Int32, &[2, 3], &[0, 0]).unwrap();
    // 2 x 3 = 6 elements expected, passing 5 should fail
    let too_few: Vec<i32> = vec![1, 2, 3, 4, 5];
    assert!(builder.set_data(&too_few).is_err());

    // Passing 7 should fail
    let too_many: Vec<i32> = vec![1, 2, 3, 4, 5, 6, 7];
    assert!(builder.set_data(&too_many).is_err());

    // Passing correct 6 elements succeeds
    let correct: Vec<i32> = vec![1, 2, 3, 4, 5, 6];
    assert!(builder.set_data(&correct).is_ok());

    // 4. Corrupted hash write rejected by engine
    let mut payload = builder.build();
    // Corrupt one data byte
    let len = payload.len();
    payload[len - 1] ^= 0xFF;

    let tmp = TestTempFile::new("corrupt_array");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();
    assert!(engine.write_array_chunk(&payload).is_err());
}
