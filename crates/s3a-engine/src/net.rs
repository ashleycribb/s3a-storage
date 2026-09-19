use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::ring_buffer::L0RingBuffer;
use crate::TelemetryRecord;

pub const S3AP_MAGIC: [u8; 4] = *b"S3AP";
pub const S3AP_VERSION: u8 = 1;
pub const S3AP_HEADER_SIZE: usize = 22;

pub const MSG_PING: u8 = 1;
pub const MSG_PONG: u8 = 2;
pub const MSG_QUERY_REQUEST: u8 = 3;
pub const MSG_QUERY_RESPONSE: u8 = 4;
pub const MSG_INGEST_REQUEST: u8 = 5;
pub const MSG_INGEST_RESPONSE: u8 = 6;
pub const MSG_ERROR: u8 = 7;

/// Sends a framed S3AP message over a TCP stream with hardware CRC32C payload checksum.
pub fn send_frame(stream: &mut TcpStream, msg_type: u8, request_id: u64, payload: &[u8]) -> io::Result<()> {
    let mut header = [0u8; S3AP_HEADER_SIZE];
    header[0..4].copy_from_slice(&S3AP_MAGIC);
    header[4] = S3AP_VERSION;
    header[5] = msg_type;
    header[6..14].copy_from_slice(&request_id.to_le_bytes());
    header[14..18].copy_from_slice(&(payload.len() as u32).to_le_bytes());

    let payload_crc = crc32c::crc32c(payload);
    header[18..22].copy_from_slice(&payload_crc.to_le_bytes());

    stream.write_all(&header)?;
    if !payload.is_empty() {
        stream.write_all(payload)?;
    }
    stream.flush()?;
    Ok(())
}

/// Reads a framed S3AP message from a TCP stream, validating magic, version, and payload CRC32C.
pub fn read_frame(stream: &mut TcpStream) -> io::Result<(u8, u64, Vec<u8>)> {
    let mut header = [0u8; S3AP_HEADER_SIZE];
    stream.read_exact(&mut header)?;

    if &header[0..4] != &S3AP_MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid S3AP magic bytes"));
    }
    if header[4] != S3AP_VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Unsupported S3AP protocol version"));
    }

    let msg_type = header[5];
    let request_id = u64::from_le_bytes(header[6..14].try_into().unwrap());
    let payload_len = u32::from_le_bytes(header[14..18].try_into().unwrap()) as usize;
    let expected_crc = u32::from_le_bytes(header[18..22].try_into().unwrap());

    // Sanity limit: max 64 MB frame
    if payload_len > 64 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "S3AP payload exceeds 64MB limit"));
    }

    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        stream.read_exact(&mut payload)?;
    }

    let actual_crc = crc32c::crc32c(&payload);
    if actual_crc != expected_crc {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("S3AP CRC mismatch: expected 0x{:08X}, got 0x{:08X}", expected_crc, actual_crc)));
    }

    Ok((msg_type, request_id, payload))
}

/// High-performance remote binary TCP client SDK for S3A.
pub struct S3AClient {
    stream: TcpStream,
    next_request_id: u64,
}

impl S3AClient {
    /// Connects to an S3A protocol server.
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        Ok(Self { stream, next_request_id: 1 })
    }

    /// Measures round-trip ping latency to the S3A server.
    pub fn ping(&mut self) -> io::Result<Duration> {
        let req_id = self.next_request_id;
        self.next_request_id += 1;

        let start = Instant::now();
        send_frame(&mut self.stream, MSG_PING, req_id, b"PING")?;

        let (msg_type, resp_id, _) = read_frame(&mut self.stream)?;
        if msg_type != MSG_PONG || resp_id != req_id {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid PING response"));
        }
        Ok(start.elapsed())
    }

    /// Executes an arbitrary S3A-QL statement remotely.
    pub fn query(&mut self, s3a_ql: &str) -> io::Result<String> {
        let req_id = self.next_request_id;
        self.next_request_id += 1;

        send_frame(&mut self.stream, MSG_QUERY_REQUEST, req_id, s3a_ql.as_bytes())?;

        let (msg_type, resp_id, payload) = read_frame(&mut self.stream)?;
        if resp_id != req_id {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Mismatched request ID"));
        }

        let resp_str = String::from_utf8(payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        if msg_type == MSG_ERROR {
            return Err(io::Error::new(io::ErrorKind::Other, resp_str));
        }

        Ok(resp_str)
    }

    /// Ingests a batch of telemetry records directly into the remote server's L0 Ring Buffer.
    pub fn insert_telemetry(&mut self, records: &[TelemetryRecord]) -> io::Result<u64> {
        let req_id = self.next_request_id;
        self.next_request_id += 1;

        let record_bytes = bytemuck::cast_slice::<TelemetryRecord, u8>(records);
        send_frame(&mut self.stream, MSG_INGEST_REQUEST, req_id, record_bytes)?;

        let (msg_type, resp_id, payload) = read_frame(&mut self.stream)?;
        if resp_id != req_id {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Mismatched request ID"));
        }

        if msg_type == MSG_ERROR {
            let err_msg = String::from_utf8_lossy(&payload);
            return Err(io::Error::new(io::ErrorKind::Other, err_msg));
        }

        if payload.len() >= 8 {
            Ok(u64::from_le_bytes(payload[0..8].try_into().unwrap()))
        } else {
            Ok(records.len() as u64)
        }
    }
}

/// Binary TCP Wire Protocol Server for S3A.
pub struct S3AProtocolServer {
    listener: TcpListener,
    running: Arc<AtomicBool>,
    ring_buffer: Option<Arc<L0RingBuffer<TelemetryRecord>>>,
    server_handle: Option<JoinHandle<()>>,
}

impl S3AProtocolServer {
    /// Binds the server to the specified address.
    pub fn bind<A: ToSocketAddrs>(addr: A, ring_buffer: Option<Arc<L0RingBuffer<TelemetryRecord>>>) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        Ok(Self {
            listener,
            running: Arc::new(AtomicBool::new(false)),
            ring_buffer,
            server_handle: None,
        })
    }

    /// Returns the local socket address this server is bound to.
    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    /// Spawns the server loop in a background thread.
    pub fn start(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            return;
        }
        self.running.store(true, Ordering::SeqCst);

        let running = Arc::clone(&self.running);
        let listener = self.listener.try_clone().unwrap();
        let ring_buffer = self.ring_buffer.clone();

        let handle = thread::spawn(move || {
            while running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if !running.load(Ordering::Relaxed) {
                            break;
                        }
                        let _ = stream.set_nodelay(true);
                        let rb = ring_buffer.clone();
                        thread::spawn(move || {
                            let _ = handle_client(&mut stream, rb);
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        self.server_handle = Some(handle);
    }

    /// Stops the server.
    pub fn stop(&mut self) {
        if self.running.swap(false, Ordering::SeqCst) {
            if let Ok(addr) = self.listener.local_addr() {
                // Wake up listener.accept()
                let _ = TcpStream::connect(addr);
            }
            if let Some(h) = self.server_handle.take() {
                let _ = h.join();
            }
        }
    }
}

impl Drop for S3AProtocolServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_client(stream: &mut TcpStream, ring_buffer: Option<Arc<L0RingBuffer<TelemetryRecord>>>) -> io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    loop {
        let (msg_type, request_id, payload) = match read_frame(stream) {
            Ok(frame) => frame,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof
                || e.kind() == io::ErrorKind::TimedOut
                || e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };

        match msg_type {
            MSG_PING => {
                send_frame(stream, MSG_PONG, request_id, b"PONG")?;
            }
            MSG_QUERY_REQUEST => {
                let ql_str = String::from_utf8_lossy(&payload);
                match crate::ql::execute_query(&ql_str) {
                    Ok(result) => {
                        let resp = format!("{:?}", result);
                        send_frame(stream, MSG_QUERY_RESPONSE, request_id, resp.as_bytes())?;
                    }
                    Err(e) => {
                        send_frame(stream, MSG_ERROR, request_id, e.as_bytes())?;
                    }
                }
            }
            MSG_INGEST_REQUEST => {
                if payload.len() % std::mem::size_of::<TelemetryRecord>() != 0 {
                    send_frame(stream, MSG_ERROR, request_id, b"Invalid TelemetryRecord batch size")?;
                    continue;
                }
                let records: &[TelemetryRecord] = bytemuck::cast_slice(&payload);
                if let Some(ref rb) = ring_buffer {
                    let mut inserted = 0u64;
                    for rec in records {
                        if rb.push(*rec).is_ok() {
                            inserted += 1;
                        }
                    }
                    send_frame(stream, MSG_INGEST_RESPONSE, request_id, &inserted.to_le_bytes())?;
                } else {
                    send_frame(stream, MSG_ERROR, request_id, b"L0 Ingestion Ring Buffer not configured on server")?;
                }
            }
            _ => {
                send_frame(stream, MSG_ERROR, request_id, b"Unknown S3AP message type")?;
            }
        }
    }
}
