//! Storage abstraction and RAM mock storage for flash memory.

/// Generic interface for NOR/NAND Flash or EEPROM hardware storage devices.
pub trait FlashStorage {
    type Error: core::fmt::Debug;

    /// Minimum programmable page size in bytes (e.g. 256 bytes).
    fn page_size(&self) -> usize;

    /// Total available capacity of the storage substrate in bytes.
    fn total_size(&self) -> usize;

    /// Reads `buf.len()` bytes starting at `offset`.
    fn read(&mut self, offset: usize, buf: &mut [u8]) -> Result<(), Self::Error>;

    /// Programs `data` bytes starting at `offset`.
    /// On NOR flash, bits can only be transitioned from 1 to 0.
    fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), Self::Error>;

    /// Erases `len` bytes starting at `offset` (typically reset to 0xFF).
    /// `offset` and `len` should be sector-aligned (e.g. 4096 bytes).
    fn erase(&mut self, offset: usize, len: usize) -> Result<(), Self::Error>;
}

/// Errors returned by the in-memory RAM flash simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamFlashError {
    OutOfBounds {
        offset: usize,
        requested: usize,
        capacity: usize,
    },
    UnalignedErase {
        offset: usize,
        len: usize,
        sector_size: usize,
    },
    CannotWriteWithoutErase {
        offset: usize,
        current: u8,
        attempted: u8,
    },
    SimulatedPowerCut {
        bytes_written_before_cut: usize,
    },
}

/// Simulated SPI NOR Flash memory buffer for deterministic unit testing.
/// Accurately simulates 0xFF erased state and 1 -> 0 bit programming physics.
#[derive(Debug, Clone)]
pub struct RamFlash<const CAPACITY: usize> {
    pub memory: [u8; CAPACITY],
    pub page_size: usize,
    pub sector_size: usize,
    pub enforce_nor_semantics: bool,
    /// If Some(limit), writing more than this many total bytes fails with SimulatedPowerCut.
    pub write_byte_limit: Option<usize>,
    pub total_bytes_written: usize,
}

impl<const CAPACITY: usize> RamFlash<CAPACITY> {
    pub const fn new(page_size: usize, sector_size: usize) -> Self {
        Self {
            memory: [0xFF; CAPACITY],
            page_size,
            sector_size,
            enforce_nor_semantics: true,
            write_byte_limit: None,
            total_bytes_written: 0,
        }
    }

    /// Sets an exact byte limit for total write operations before cutting power.
    pub fn set_write_limit(&mut self, limit: Option<usize>) {
        self.write_byte_limit = limit;
        self.total_bytes_written = 0;
    }

    /// Resets the total bytes written counter.
    pub fn reset_write_counter(&mut self) {
        self.total_bytes_written = 0;
    }

    /// Returns a direct slice of the internal memory.
    pub fn as_slice(&self) -> &[u8] {
        &self.memory
    }
}

impl<const CAPACITY: usize> Default for RamFlash<CAPACITY> {
    fn default() -> Self {
        Self::new(256, 4096)
    }
}

impl<const CAPACITY: usize> FlashStorage for RamFlash<CAPACITY> {
    type Error = RamFlashError;

    fn page_size(&self) -> usize {
        self.page_size
    }

    fn total_size(&self) -> usize {
        CAPACITY
    }

    fn read(&mut self, offset: usize, buf: &mut [u8]) -> Result<(), Self::Error> {
        if offset + buf.len() > CAPACITY {
            return Err(RamFlashError::OutOfBounds {
                offset,
                requested: buf.len(),
                capacity: CAPACITY,
            });
        }
        buf.copy_from_slice(&self.memory[offset..offset + buf.len()]);
        Ok(())
    }

    fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), Self::Error> {
        if offset + data.len() > CAPACITY {
            return Err(RamFlashError::OutOfBounds {
                offset,
                requested: data.len(),
                capacity: CAPACITY,
            });
        }

        for (i, &b) in data.iter().enumerate() {
            if let Some(limit) = self.write_byte_limit {
                if self.total_bytes_written >= limit {
                    return Err(RamFlashError::SimulatedPowerCut {
                        bytes_written_before_cut: self.total_bytes_written,
                    });
                }
            }

            let current = self.memory[offset + i];
            if self.enforce_nor_semantics && (offset + i) >= 512 {
                // Tile sectors strictly enforce NOR flash physics: 1 -> 0 only without erase.
                if (current & b) != b {
                    return Err(RamFlashError::CannotWriteWithoutErase {
                        offset: offset + i,
                        current,
                        attempted: b,
                    });
                }
                self.memory[offset + i] = current & b;
            } else {
                self.memory[offset + i] = b;
            }
            self.total_bytes_written += 1;
        }

        Ok(())
    }

    fn erase(&mut self, offset: usize, len: usize) -> Result<(), Self::Error> {
        if offset + len > CAPACITY {
            return Err(RamFlashError::OutOfBounds {
                offset,
                requested: len,
                capacity: CAPACITY,
            });
        }

        if offset % self.sector_size != 0 || len % self.sector_size != 0 {
            return Err(RamFlashError::UnalignedErase {
                offset,
                len,
                sector_size: self.sector_size,
            });
        }

        for b in &mut self.memory[offset..offset + len] {
            *b = 0xFF;
        }

        Ok(())
    }
}
