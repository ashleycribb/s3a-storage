//! Quantized Slab Storage Engine & Extent Manager (§4).
//!
//! Manages size-segregated extent files (`.slab`) on disk across 10 quantized
//! size classes (4 KB to 128 KB with 1.5x stepping), featuring:
//! - 4096-byte `SlabExtentHeader` with magic `b"S3ASLAB1"`.
//! - Slot allocation and deallocation tracked via a compact in-header bitmap.
//! - Deterministic $O(1)$ offset calculation (`4096 + slot_index * slot_size_bytes`).
//! - Automatic extent rollover when an extent file is full.
//! - Small LRU open file descriptor pool (`ExtentFdPool`) to avoid handle exhaustion.

use bytemuck::{bytes_of, bytes_of_mut, Pod, Zeroable};
use s3a_core::{
    select_size_class, size_class_bytes, MAX_SIZE_CLASS_ID, NUM_SLAB_CLASSES,
    MAX_SLAB_PAYLOAD_SIZE, TileAddress, TileAddressError,
};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Magic tag identifying a valid S3A slab extent file (`S3ASLAB1`).
pub const SLAB_MAGIC: [u8; 8] = *b"S3ASLAB1";

/// Byte length of the 8-byte aligned extent header sector.
pub const SLAB_HEADER_SIZE: usize = 4096;

/// Number of bytes allocated for the slot occupancy bitmap in the header.
/// 4072 bytes * 8 bits = 32,576 slots per extent file.
pub const SLAB_BITMAP_BYTES: usize = 4072;

/// Maximum number of slots supported in a single extent file.
pub const MAX_SLOTS_PER_EXTENT: u32 = (SLAB_BITMAP_BYTES * 8) as u32;

/// Default slot count allocated per extent when not explicitly configured.
pub const DEFAULT_SLOTS_PER_EXTENT: u32 = 1024;

/// Default maximum number of open file descriptors retained in the LRU pool.
pub const DEFAULT_FD_POOL_CAPACITY: usize = 32;

/// Errors arising during slab allocation, extent I/O, or addressing operations.
#[derive(Debug)]
pub enum SlabError {
    InvalidMagic,
    InvalidSizeClass(u8),
    InvalidSlotCount(u32),
    SlotIndexOutOfBounds { slot_index: u32, slot_count: u32 },
    SlotNotAllocated(u32),
    PayloadTooLarge { payload_len: usize, max_capacity: usize },
    DataExceedsSlotSize { data_len: usize, slot_size: usize },
    Address(TileAddressError),
    Io(io::Error),
}

impl core::fmt::Display for SlabError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "Invalid slab extent file magic (expected b\"S3ASLAB1\")"),
            Self::InvalidSizeClass(c) => write!(f, "Invalid size class ID {c}"),
            Self::InvalidSlotCount(c) => write!(f, "Invalid slot count {c}"),
            Self::SlotIndexOutOfBounds {
                slot_index,
                slot_count,
            } => {
                write!(f, "Slot index {slot_index} out of bounds for extent with {slot_count} slots")
            }
            Self::SlotNotAllocated(s) => write!(f, "Slot {s} is not currently allocated"),
            Self::PayloadTooLarge {
                payload_len,
                max_capacity,
            } => {
                write!(f, "Payload length {payload_len} exceeds maximum slab capacity {max_capacity}")
            }
            Self::DataExceedsSlotSize {
                data_len,
                slot_size,
            } => {
                write!(f, "Data length {data_len} exceeds slot capacity {slot_size}")
            }
            Self::Address(e) => write!(f, "TileAddress error: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for SlabError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Address(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for SlabError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<TileAddressError> for SlabError {
    fn from(e: TileAddressError) -> Self {
        Self::Address(e)
    }
}

/// 4096-byte hardware-aligned extent file header (POD / Zeroable).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlabExtentHeader {
    pub magic: [u8; 8],
    pub extent_id: u32,
    pub size_class_id: u8,
    pub _reserved: [u8; 3],
    pub slot_count: u32,
    pub slot_size_bytes: u32,
    pub allocation_bitmap: [u8; SLAB_BITMAP_BYTES],
}

// Safety: SlabExtentHeader contains only plain integer types and byte arrays with
// no padding bytes, has an exact 4096-byte size, and is valid for any bit pattern.
unsafe impl Zeroable for SlabExtentHeader {}
unsafe impl Pod for SlabExtentHeader {}

impl SlabExtentHeader {
    /// Creates a new unallocated extent header for the specified size class.
    pub fn new(size_class_id: u8, extent_id: u32, slot_count: u32) -> Result<Self, SlabError> {
        if size_class_id > MAX_SIZE_CLASS_ID {
            return Err(SlabError::InvalidSizeClass(size_class_id));
        }
        let slot_size_bytes = size_class_bytes(size_class_id)
            .ok_or(SlabError::InvalidSizeClass(size_class_id))?;
        if slot_count == 0 || slot_count > MAX_SLOTS_PER_EXTENT {
            return Err(SlabError::InvalidSlotCount(slot_count));
        }

        Ok(Self {
            magic: SLAB_MAGIC,
            extent_id,
            size_class_id,
            _reserved: [0; 3],
            slot_count,
            slot_size_bytes,
            allocation_bitmap: [0; SLAB_BITMAP_BYTES],
        })
    }

    /// Checks if a slot index is currently marked as allocated.
    pub fn is_slot_allocated(&self, slot_index: u32) -> Result<bool, SlabError> {
        if slot_index >= self.slot_count {
            return Err(SlabError::SlotIndexOutOfBounds {
                slot_index,
                slot_count: self.slot_count,
            });
        }
        let byte_idx = (slot_index / 8) as usize;
        let bit_idx = (slot_index % 8) as u8;
        Ok((self.allocation_bitmap[byte_idx] & (1 << bit_idx)) != 0)
    }

    /// Sets or clears the allocation bit for a slot index.
    pub fn set_slot_allocated(&mut self, slot_index: u32, allocated: bool) -> Result<(), SlabError> {
        if slot_index >= self.slot_count {
            return Err(SlabError::SlotIndexOutOfBounds {
                slot_index,
                slot_count: self.slot_count,
            });
        }
        let byte_idx = (slot_index / 8) as usize;
        let bit_idx = (slot_index % 8) as u8;
        if allocated {
            self.allocation_bitmap[byte_idx] |= 1 << bit_idx;
        } else {
            self.allocation_bitmap[byte_idx] &= !(1 << bit_idx);
        }
        Ok(())
    }

    /// Finds the first unallocated slot in the extent, if any.
    pub fn find_first_free_slot(&self) -> Option<u32> {
        let max_bytes = ((self.slot_count + 7) / 8) as usize;
        for (byte_idx, &byte) in self.allocation_bitmap[..max_bytes].iter().enumerate() {
            if byte != 0xFF {
                for bit_idx in 0..8 {
                    if (byte & (1 << bit_idx)) == 0 {
                        let slot = (byte_idx as u32 * 8) + bit_idx as u32;
                        if slot < self.slot_count {
                            return Some(slot);
                        }
                    }
                }
            }
        }
        None
    }

    /// Returns the total number of currently allocated slots in this extent.
    pub fn allocated_count(&self) -> u32 {
        let max_bytes = ((self.slot_count + 7) / 8) as usize;
        let mut count = 0;
        for (byte_idx, &byte) in self.allocation_bitmap[..max_bytes].iter().enumerate() {
            if byte != 0 {
                for bit_idx in 0..8 {
                    let slot = (byte_idx as u32 * 8) + bit_idx as u32;
                    if slot < self.slot_count && (byte & (1 << bit_idx)) != 0 {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// Returns true if all slots in this extent are occupied.
    pub fn is_full(&self) -> bool {
        self.allocated_count() >= self.slot_count
    }

    /// Validates the magic and structure of an extent header read from disk.
    pub fn validate(&self) -> Result<(), SlabError> {
        if self.magic != SLAB_MAGIC {
            return Err(SlabError::InvalidMagic);
        }
        if self.size_class_id > MAX_SIZE_CLASS_ID {
            return Err(SlabError::InvalidSizeClass(self.size_class_id));
        }
        let expected_size = size_class_bytes(self.size_class_id)
            .ok_or(SlabError::InvalidSizeClass(self.size_class_id))?;
        if self.slot_size_bytes != expected_size {
            return Err(SlabError::InvalidSizeClass(self.size_class_id));
        }
        if self.slot_count == 0 || self.slot_count > MAX_SLOTS_PER_EXTENT {
            return Err(SlabError::InvalidSlotCount(self.slot_count));
        }
        Ok(())
    }
}

/// Identifies an open extent file within the LRU pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExtentKey {
    pub size_class_id: u8,
    pub extent_id: u32,
}

/// LRU pool of open file descriptors to bound active handle counts across thousands of extents.
pub struct ExtentFdPool {
    capacity: usize,
    entries: HashMap<ExtentKey, File>,
    lru_order: VecDeque<ExtentKey>,
}

impl ExtentFdPool {
    /// Creates a new LRU pool with the given capacity of open file descriptors.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            lru_order: VecDeque::new(),
        }
    }

    /// Capacity limit of open file descriptors.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current count of open file descriptors in the pool.
    pub fn open_count(&self) -> usize {
        self.entries.len()
    }

    /// Retrieves an existing file descriptor or opens a new one, evicting the LRU file if full.
    pub fn get_or_open(&mut self, key: ExtentKey, path: &Path) -> io::Result<&mut File> {
        if self.entries.contains_key(&key) {
            if let Some(pos) = self.lru_order.iter().position(|&k| k == key) {
                self.lru_order.remove(pos);
            }
            self.lru_order.push_back(key);
            return Ok(self.entries.get_mut(&key).unwrap());
        }

        if self.entries.len() >= self.capacity {
            if let Some(evicted_key) = self.lru_order.pop_front() {
                if let Some(mut file) = self.entries.remove(&evicted_key) {
                    let _ = file.flush();
                }
            }
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        self.entries.insert(key, file);
        self.lru_order.push_back(key);
        Ok(self.entries.get_mut(&key).unwrap())
    }

    /// Flushes all open file descriptors currently held in the pool.
    pub fn flush_all(&mut self) -> io::Result<()> {
        for file in self.entries.values_mut() {
            file.flush()?;
        }
        Ok(())
    }

    /// Closes and evicts a specific key from the pool.
    pub fn remove(&mut self, key: &ExtentKey) {
        if let Some(mut file) = self.entries.remove(key) {
            let _ = file.flush();
        }
        if let Some(pos) = self.lru_order.iter().position(|k| k == key) {
            self.lru_order.remove(pos);
        }
    }
}

/// Quantized slab extent allocator managing storage directories across 10 size classes.
pub struct QuantizedSlabManager {
    base_dir: PathBuf,
    slots_per_extent: u32,
    fd_pool: ExtentFdPool,
    active_extents: HashMap<u8, u32>,
}

impl QuantizedSlabManager {
    /// Initializes a new slab manager anchored at `base_dir` with default pool and extent sizes.
    pub fn new<P: AsRef<Path>>(base_dir: P) -> io::Result<Self> {
        Self::with_options(base_dir, DEFAULT_FD_POOL_CAPACITY, DEFAULT_SLOTS_PER_EXTENT)
    }

    /// Initializes a slab manager with custom FD pool capacity and slots per extent.
    pub fn with_options<P: AsRef<Path>>(
        base_dir: P,
        pool_capacity: usize,
        slots_per_extent: u32,
    ) -> io::Result<Self> {
        let base_dir = base_dir.as_ref().to_path_buf();
        fs::create_dir_all(&base_dir)?;

        let mut active_extents = HashMap::new();

        // Scan existing directories for highest extent ID per size class
        for class_id in 0..NUM_SLAB_CLASSES as u8 {
            let class_dir = base_dir.join(format!("slabs/class_{class_id}"));
            if class_dir.is_dir() {
                let mut max_extent = 0u32;
                let mut found_any = false;
                if let Ok(entries) = fs::read_dir(&class_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with("extent_") && name_str.ends_with(".slab") {
                            let hex_part = &name_str[7..name_str.len() - 5];
                            if let Ok(id) = u32::from_str_radix(hex_part, 16) {
                                max_extent = max_extent.max(id);
                                found_any = true;
                            }
                        }
                    }
                }
                if found_any {
                    active_extents.insert(class_id, max_extent);
                }
            }
        }

        Ok(Self {
            base_dir,
            slots_per_extent: slots_per_extent.min(MAX_SLOTS_PER_EXTENT).max(1),
            fd_pool: ExtentFdPool::new(pool_capacity),
            active_extents,
        })
    }

    /// Base directory where slab storage is managed.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Configured slot count per extent.
    pub fn slots_per_extent(&self) -> u32 {
        self.slots_per_extent
    }

    /// Returns the filesystem path for a specific extent file:
    /// `<base_dir>/slabs/class_{class_id}/extent_{extent_id:07x}.slab`
    pub fn extent_path(&self, class_id: u8, extent_id: u32) -> PathBuf {
        self.base_dir
            .join(format!("slabs/class_{class_id}"))
            .join(format!("extent_{extent_id:07x}.slab"))
    }

    /// Allocates the next available slot that can fit `payload_len` bytes.
    ///
    /// Automatically selects the smallest matching quantized size class, finds an unallocated
    /// slot in the active extent, or creates a new rolled-over extent if the active one is full.
    pub fn allocate_slot(&mut self, payload_len: usize) -> Result<TileAddress, SlabError> {
        let class_id = select_size_class(payload_len).ok_or(SlabError::PayloadTooLarge {
            payload_len,
            max_capacity: MAX_SLAB_PAYLOAD_SIZE,
        })?;

        let mut extent_id = *self.active_extents.entry(class_id).or_insert(0);

        loop {
            let path = self.extent_path(class_id, extent_id);
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut header = SlabExtentHeader::new(class_id, extent_id, self.slots_per_extent)?;
                header.set_slot_allocated(0, true)?;

                let key = ExtentKey {
                    size_class_id: class_id,
                    extent_id,
                };
                let file = self.fd_pool.get_or_open(key, &path)?;
                file.seek(SeekFrom::Start(0))?;
                file.write_all(bytes_of(&header))?;
                file.flush()?;

                self.active_extents.insert(class_id, extent_id);
                return Ok(TileAddress::new(class_id, extent_id, 0)?);
            }

            // Read existing header
            let key = ExtentKey {
                size_class_id: class_id,
                extent_id,
            };
            let file = self.fd_pool.get_or_open(key, &path)?;
            file.seek(SeekFrom::Start(0))?;
            let mut header = SlabExtentHeader::zeroed();
            file.read_exact(bytes_of_mut(&mut header))?;
            header.validate()?;

            if let Some(free_slot) = header.find_first_free_slot() {
                header.set_slot_allocated(free_slot, true)?;
                file.seek(SeekFrom::Start(0))?;
                file.write_all(bytes_of(&header))?;
                file.flush()?;

                return Ok(TileAddress::new(class_id, extent_id, free_slot)?);
            }

            // Extent is full: rollover to next extent ID
            extent_id += 1;
            self.active_extents.insert(class_id, extent_id);
        }
    }

    /// Frees an allocated slot in the specified extent, marking its bit available for reuse.
    pub fn free_slot(&mut self, address: TileAddress) -> Result<(), SlabError> {
        let class_id = address.size_class_id();
        let extent_id = address.extent_id();
        let slot_index = address.slot_index();

        let path = self.extent_path(class_id, extent_id);
        if !path.exists() {
            return Err(SlabError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Extent file {path:?} does not exist"),
            )));
        }

        let key = ExtentKey {
            size_class_id: class_id,
            extent_id,
        };
        let file = self.fd_pool.get_or_open(key, &path)?;
        file.seek(SeekFrom::Start(0))?;
        let mut header = SlabExtentHeader::zeroed();
        file.read_exact(bytes_of_mut(&mut header))?;
        header.validate()?;

        if !header.is_slot_allocated(slot_index)? {
            return Err(SlabError::SlotNotAllocated(slot_index));
        }

        header.set_slot_allocated(slot_index, false)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(bytes_of(&header))?;
        file.flush()?;

        Ok(())
    }

    /// Writes raw payload bytes directly into the specified physical slot.
    pub fn write_slot(&mut self, address: TileAddress, data: &[u8]) -> Result<(), SlabError> {
        let class_id = address.size_class_id();
        let slot_size = size_class_bytes(class_id).ok_or(SlabError::InvalidSizeClass(class_id))? as usize;
        if data.len() > slot_size {
            return Err(SlabError::DataExceedsSlotSize {
                data_len: data.len(),
                slot_size,
            });
        }

        let offset = address.physical_offset()?;
        let path = self.extent_path(class_id, address.extent_id());

        let key = ExtentKey {
            size_class_id: class_id,
            extent_id: address.extent_id(),
        };
        let file = self.fd_pool.get_or_open(key, &path)?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(data)?;
        file.flush()?;

        Ok(())
    }

    /// Reads raw payload bytes from the specified physical slot into `out`.
    /// Returns the number of bytes read (bounded by slot size, file length, and `out.len()`).
    pub fn read_slot(&mut self, address: TileAddress, out: &mut [u8]) -> Result<usize, SlabError> {
        let class_id = address.size_class_id();
        let slot_size = size_class_bytes(class_id).ok_or(SlabError::InvalidSizeClass(class_id))? as usize;
        let to_read = slot_size.min(out.len());

        let offset = address.physical_offset()?;
        let path = self.extent_path(class_id, address.extent_id());

        let key = ExtentKey {
            size_class_id: class_id,
            extent_id: address.extent_id(),
        };
        let file = self.fd_pool.get_or_open(key, &path)?;
        let file_len = file.metadata()?.len();
        if offset >= file_len {
            return Ok(0);
        }
        let available = (file_len - offset) as usize;
        let to_read = to_read.min(available);
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut out[..to_read])?;

        Ok(to_read)
    }

    /// Reads and returns the header of an extent file.
    pub fn read_header(&mut self, class_id: u8, extent_id: u32) -> Result<SlabExtentHeader, SlabError> {
        let path = self.extent_path(class_id, extent_id);
        let key = ExtentKey {
            size_class_id: class_id,
            extent_id,
        };
        let file = self.fd_pool.get_or_open(key, &path)?;
        file.seek(SeekFrom::Start(0))?;
        let mut header = SlabExtentHeader::zeroed();
        file.read_exact(bytes_of_mut(&mut header))?;
        header.validate()?;
        Ok(header)
    }
}
