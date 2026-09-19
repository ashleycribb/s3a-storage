use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Backpressure policy when the L0 ring buffer reaches capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressurePolicy {
    /// Return an error immediately (`Err(IngestError::BufferFull)`).
    Error,
    /// Overwrite the oldest record in the buffer without blocking, incrementing the drop counter.
    DropOldest,
    /// Block the producer thread until the background compactor drains space.
    Block,
}

/// Errors returned by L0 Ring Buffer operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    BufferFull,
    Closed,
    Timeout,
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferFull => write!(f, "L0 Ring Buffer is full"),
            Self::Closed => write!(f, "L0 Ring Buffer is closed"),
            Self::Timeout => write!(f, "L0 Ring Buffer push timed out"),
        }
    }
}

impl std::error::Error for IngestError {}

struct RingBufferState<T> {
    data: Vec<T>,
    head: usize,
    tail: usize,
    count: usize,
    capacity: usize,
    dropped_count: u64,
    closed: bool,
}

/// High-throughput, thread-safe L0 Ingestion Ring Buffer for real-time sensor streams and telemetry.
/// Serves as the in-memory "delta overlay" for sub-millisecond certified freshness.
pub struct L0RingBuffer<T> {
    state: Mutex<RingBufferState<T>>,
    not_full: Condvar,
    not_empty: Condvar,
    policy: BackpressurePolicy,
}

impl<T: Copy + Default + Send + 'static> L0RingBuffer<T> {
    /// Creates a new L0 Ring Buffer with the specified capacity and backpressure policy.
    pub fn new(capacity: usize, policy: BackpressurePolicy) -> Self {
        assert!(capacity > 0, "Capacity must be greater than zero");
        Self {
            state: Mutex::new(RingBufferState {
                data: vec![T::default(); capacity],
                head: 0,
                tail: 0,
                count: 0,
                capacity,
                dropped_count: 0,
                closed: false,
            }),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
            policy,
        }
    }

    /// Pushes a record into the L0 ring buffer according to the configured backpressure policy.
    pub fn push(&self, record: T) -> Result<(), IngestError> {
        let mut state = self.state.lock().unwrap();

        loop {
            if state.closed {
                return Err(IngestError::Closed);
            }

            if state.count < state.capacity {
                let tail = state.tail;
                state.data[tail] = record;
                state.tail = (tail + 1) % state.capacity;
                state.count += 1;
                self.not_empty.notify_one();
                return Ok(());
            }

            match self.policy {
                BackpressurePolicy::Error => {
                    return Err(IngestError::BufferFull);
                }
                BackpressurePolicy::DropOldest => {
                    // Overwrite oldest entry at head
                    let head = state.head;
                    let tail = state.tail;
                    state.data[tail] = record;
                    state.tail = (tail + 1) % state.capacity;
                    state.head = (head + 1) % state.capacity;
                    state.dropped_count += 1;
                    self.not_empty.notify_one();
                    return Ok(());
                }
                BackpressurePolicy::Block => {
                    state = self.not_full.wait(state).unwrap();
                }
            }
        }
    }

    /// Pushes a record with a maximum wait timeout if `policy == Block`.
    pub fn push_timeout(&self, record: T, timeout: Duration) -> Result<(), IngestError> {
        let mut state = self.state.lock().unwrap();
        let deadline = Instant::now() + timeout;

        loop {
            if state.closed {
                return Err(IngestError::Closed);
            }

            if state.count < state.capacity {
                let tail = state.tail;
                state.data[tail] = record;
                state.tail = (tail + 1) % state.capacity;
                state.count += 1;
                self.not_empty.notify_one();
                return Ok(());
            }

            match self.policy {
                BackpressurePolicy::Error => return Err(IngestError::BufferFull),
                BackpressurePolicy::DropOldest => {
                    let head = state.head;
                    let tail = state.tail;
                    state.data[tail] = record;
                    state.tail = (tail + 1) % state.capacity;
                    state.head = (head + 1) % state.capacity;
                    state.dropped_count += 1;
                    self.not_empty.notify_one();
                    return Ok(());
                }
                BackpressurePolicy::Block => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(IngestError::Timeout);
                    }
                    let remaining = deadline - now;
                    let (s, wait_res) = self.not_full.wait_timeout(state, remaining).unwrap();
                    state = s;
                    if wait_res.timed_out() && state.count >= state.capacity {
                        return Err(IngestError::Timeout);
                    }
                }
            }
        }
    }

    /// Drains up to `max_items` records from the buffer for background flushing or compaction.
    pub fn drain(&self, max_items: usize) -> Vec<T> {
        let mut state = self.state.lock().unwrap();
        if state.count == 0 {
            return Vec::new();
        }

        let to_drain = state.count.min(max_items);
        let mut drained = Vec::with_capacity(to_drain);

        for _ in 0..to_drain {
            let head = state.head;
            drained.push(state.data[head]);
            state.head = (head + 1) % state.capacity;
            state.count -= 1;
        }

        if to_drain > 0 {
            self.not_full.notify_all();
        }

        drained
    }

    /// Drains all available records from the buffer.
    pub fn drain_all(&self) -> Vec<T> {
        let count = self.len();
        self.drain(count)
    }

    /// Peeks all current records without removing them.
    /// Used by the query engine ("delta overlay") to provide certified real-time freshness.
    pub fn snapshot(&self) -> Vec<T> {
        let state = self.state.lock().unwrap();
        let mut records = Vec::with_capacity(state.count);
        let mut curr = state.head;
        for _ in 0..state.count {
            records.push(state.data[curr]);
            curr = (curr + 1) % state.capacity;
        }
        records
    }

    /// Returns the number of unread records currently in the buffer.
    pub fn len(&self) -> usize {
        self.state.lock().unwrap().count
    }

    /// Returns true if the buffer has no records.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.state.lock().unwrap().capacity
    }

    /// Returns the count of dropped records due to `DropOldest` backpressure.
    pub fn dropped_count(&self) -> u64 {
        self.state.lock().unwrap().dropped_count
    }

    /// Closes the ring buffer, waking any waiting threads and rejecting further pushes.
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }

    /// Returns true if the buffer is closed.
    pub fn is_closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }
}
