//! Codec for OGC GeoPose 1.0 Basic-Quaternion JSON (doc 21-056r11, clause 9.2.3).
//!
//! Written from scratch in Rust from the specification; no code is copied from
//! any OSCP / OpenARCloud repository. The parser follows the Permissive
//! encoding (§9.2.3.2, the default GeoPose): `position` and `quaternion` with
//! their seven numeric fields are required, and any additional JSON properties
//! are ignored. Basic carries no `validTime`; a `validTime` property, if
//! present, is ignored rather than stored.
//!
//! Number tokens are parsed with Rust's `str::parse::<f64>()` (correctly
//! rounded) instead of serde_json: serde_json 1.0.154 mis-parses one of the
//! seven OGC example values by 1 ULP — `"-0.9050852994339209"` comes out as
//! bits `bfecf675722516ff` (`-0.9050852994339208`) where the correctly-rounded
//! value is bits `bfecf67572251700` (`-0.9050852994339209`), as confirmed
//! against Python's `float()` and the Rust literal, which agree. AT-SP-01
//! demands exact f64 values, so the codec keeps its own minimal JSON reader
//! (below) and converts number tokens itself.

use crate::records::GeoposeBasicQuatRecord;
use sha2::{Digest, Sha256};

/// Errors from encoding or decoding a Basic-Quaternion record.
#[derive(Debug, Clone, PartialEq)]
pub enum GeoposeError {
    /// The bytes are not parseable JSON.
    InvalidJson(String),
    /// A required object (`position` / `quaternion`) is absent.
    MissingField(&'static str),
    /// A required numeric field is absent or is not a JSON number.
    NonNumericField(&'static str),
    /// A required numeric field parses to a non-finite f64 (e.g. `1e999`).
    NonFiniteField(&'static str),
    /// `source_id` was all zero: a record must name its source.
    MissingSourceId,
    /// `origin_ptr` was all zero: a record must point at its original file.
    MissingOriginPtr,
    /// The quaternion is not unit within the stated tolerance.
    NonUnitQuaternion {
        /// Observed x²+y²+z²+w².
        norm_sq: f64,
        /// Tolerance the caller stated.
        tolerance: f64,
    },
    /// A decoded record carries a non-zero `_pad` (structural invariant).
    NonZeroPadding,
}

impl std::fmt::Display for GeoposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeoposeError::InvalidJson(e) => write!(f, "invalid GeoPose JSON: {}", e),
            GeoposeError::MissingField(field) => write!(f, "missing GeoPose field: {}", field),
            GeoposeError::NonNumericField(field) => {
                write!(f, "GeoPose field is not a number: {}", field)
            }
            GeoposeError::NonFiniteField(field) => {
                write!(f, "GeoPose field is not finite: {}", field)
            }
            GeoposeError::MissingSourceId => write!(f, "source_id is all zero"),
            GeoposeError::MissingOriginPtr => write!(f, "origin_ptr is all zero"),
            GeoposeError::NonUnitQuaternion { norm_sq, tolerance } => write!(
                f,
                "quaternion is not unit: norm-squared {} (tolerance {})",
                norm_sq, tolerance
            ),
            GeoposeError::NonZeroPadding => write!(f, "record _pad is non-zero"),
        }
    }
}

impl std::error::Error for GeoposeError {}

/// A decoded Basic-Quaternion pose: the seven f64 values of the record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodedGeopose {
    pub lat: f64,
    pub lon: f64,
    pub h: f64,
    pub qx: f64,
    pub qy: f64,
    pub qz: f64,
    pub qw: f64,
}

// ---------------------------------------------------------------------------
// Minimal JSON reader (fixed-schema use only).
//
// Parses one JSON document into a small value tree. Numbers are kept as raw
// token slices so the caller can convert them with `str::parse::<f64>()`.
// Strings are decoded (escapes and \uXXXX, including surrogate pairs).
// Nesting is capped at 64 to bound recursion on adversarial input.
// ---------------------------------------------------------------------------

const MAX_DEPTH: usize = 64;

#[derive(Debug)]
enum JsonValue<'a> {
    Null,
    Bool(bool),
    Number(&'a str),
    Str(String),
    Array(Vec<JsonValue<'a>>),
    Object(Vec<(String, JsonValue<'a>)>),
}

struct JsonReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn err<T>(&self, msg: &'static str) -> Result<T, GeoposeError> {
        Err(GeoposeError::InvalidJson(msg.to_string()))
    }

    fn expect(&mut self, b: u8) -> Result<(), GeoposeError> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            self.err("unexpected character")
        }
    }

    fn literal(&mut self, text: &str) -> Result<(), GeoposeError> {
        if self.bytes[self.pos..].starts_with(text.as_bytes()) {
            self.pos += text.len();
            Ok(())
        } else {
            self.err("invalid literal")
        }
    }

    fn parse_document(&mut self) -> Result<JsonValue<'a>, GeoposeError> {
        let value = self.parse_value(0)?;
        self.ws();
        if self.pos != self.bytes.len() {
            return self.err("trailing data after JSON value");
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonValue<'a>, GeoposeError> {
        if depth > MAX_DEPTH {
            return self.err("JSON nesting too deep");
        }
        self.ws();
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(JsonValue::Str(self.parse_string()?)),
            Some(b't') => {
                self.literal("true")?;
                Ok(JsonValue::Bool(true))
            }
            Some(b'f') => {
                self.literal("false")?;
                Ok(JsonValue::Bool(false))
            }
            Some(b'n') => {
                self.literal("null")?;
                Ok(JsonValue::Null)
            }
            Some(b'-') | Some(b'0'..=b'9') => Ok(JsonValue::Number(self.parse_number()?)),
            _ => self.err("unexpected character"),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonValue<'a>, GeoposeError> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(fields));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return self.err("object key must be a string");
            }
            let key = self.parse_string()?;
            self.ws();
            self.expect(b':')?;
            let value = self.parse_value(depth + 1)?;
            fields.push((key, value));
            self.ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(fields));
                }
                _ => return self.err("expected ',' or '}' in object"),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonValue<'a>, GeoposeError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value(depth + 1)?);
            self.ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return self.err("expected ',' or ']' in array"),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, GeoposeError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(b) = self.peek() else {
                return self.err("unterminated string");
            };
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let Some(e) = self.peek() else {
                        return self.err("unterminated escape");
                    };
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let cp = self.parse_hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                // High surrogate: expect a low surrogate.
                                if self.peek() == Some(b'\\')
                                    && self.bytes.get(self.pos + 1) == Some(&b'u')
                                {
                                    self.pos += 2;
                                    let lo = self.parse_hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                        out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                                    } else {
                                        return self.err("invalid low surrogate");
                                    }
                                } else {
                                    return self.err("unpaired high surrogate");
                                }
                            } else if (0xDC00..0xE000).contains(&cp) {
                                return self.err("unpaired low surrogate");
                            } else {
                                out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                            }
                        }
                        _ => return self.err("invalid escape"),
                    }
                }
                0x00..=0x1F => return self.err("control character in string"),
                _ => {
                    // Copy one UTF-8 sequence verbatim.
                    let start = self.pos;
                    let len = utf8_len(b);
                    if self.pos + len > self.bytes.len() {
                        return self.err("invalid UTF-8 in string");
                    }
                    let s = std::str::from_utf8(&self.bytes[start..start + len])
                        .map_err(|_| GeoposeError::InvalidJson("invalid UTF-8".to_string()))?;
                    out.push_str(s);
                    self.pos += len;
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, GeoposeError> {
        let mut cp: u32 = 0;
        for _ in 0..4 {
            let Some(b) = self.peek() else {
                return self.err("truncated \\u escape");
            };
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u32,
                b'a'..=b'f' => (b - b'a' + 10) as u32,
                b'A'..=b'F' => (b - b'A' + 10) as u32,
                _ => return self.err("invalid hex digit in \\u escape"),
            };
            cp = (cp << 4) | digit;
            self.pos += 1;
        }
        Ok(cp)
    }

    /// Parses a JSON number token (`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`)
    /// and returns the raw token slice (conversion is the caller's job).
    fn parse_number(&mut self) -> Result<&'a str, GeoposeError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return self.err("invalid number"),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return self.err("invalid number fraction");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return self.err("invalid number exponent");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| GeoposeError::InvalidJson("invalid number".to_string()))
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn object_field<'a>(obj: &'a JsonValue<'a>, field: &'static str) -> Result<&'a JsonValue<'a>, GeoposeError> {
    match obj {
        JsonValue::Object(fields) => fields
            .iter()
            .rev() // Last duplicate wins, matching serde_json.
            .find(|(k, _)| k == field)
            .map(|(_, v)| v)
            .ok_or(GeoposeError::MissingField(field)),
        _ => Err(GeoposeError::MissingField(field)),
    }
}

fn number_field(obj: &JsonValue<'_>, field: &'static str) -> Result<f64, GeoposeError> {
    let value = object_field(obj, field)?;
    match value {
        JsonValue::Number(token) => {
            let parsed: f64 = token
                .parse()
                .map_err(|_| GeoposeError::NonNumericField(field))?;
            if !parsed.is_finite() {
                return Err(GeoposeError::NonFiniteField(field));
            }
            Ok(parsed)
        }
        _ => Err(GeoposeError::NonNumericField(field)),
    }
}

/// Encodes OGC GeoPose Basic-Quaternion JSON into a 128 B record.
///
/// * `json` — the original JSON bytes; `content_hash` is their SHA-256.
/// * `source_id` / `origin_ptr` — caller-provided provenance; all-zero is rejected.
/// * `unit_tolerance` — the caller-stated tolerance on |x²+y²+z²+w² − 1|.
///   The tolerance lives with the caller (the test states it), never in the codec.
pub fn encode_geopose_basic_quat(
    json: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    unit_tolerance: f64,
) -> Result<GeoposeBasicQuatRecord, GeoposeError> {
    if source_id == [0u8; 16] {
        return Err(GeoposeError::MissingSourceId);
    }
    if origin_ptr == [0u8; 16] {
        return Err(GeoposeError::MissingOriginPtr);
    }
    let doc = JsonReader::new(json).parse_document()?;
    let position = object_field(&doc, "position")?;
    let quaternion = object_field(&doc, "quaternion")?;
    let lat = number_field(position, "lat")?;
    let lon = number_field(position, "lon")?;
    let h = number_field(position, "h")?;
    let qx = number_field(quaternion, "x")?;
    let qy = number_field(quaternion, "y")?;
    let qz = number_field(quaternion, "z")?;
    let qw = number_field(quaternion, "w")?;

    let record = GeoposeBasicQuatRecord {
        source_id,
        content_hash: Sha256::digest(json).into(),
        origin_ptr,
        lat,
        lon,
        h,
        qx,
        qy,
        qz,
        qw,
        _pad: 0,
    };
    let norm_sq = record.norm_sq();
    if (norm_sq - 1.0).abs() > unit_tolerance {
        return Err(GeoposeError::NonUnitQuaternion {
            norm_sq,
            tolerance: unit_tolerance,
        });
    }
    Ok(record)
}

/// Decodes a record into its seven f64 values.
///
/// Rejects a record whose `_pad` is non-zero (the pad is a structural
/// invariant of the layout, not data). The unit-norm check is the encoder's
/// gate (AT-SP-03); decode does not re-check it — see the test log.
pub fn decode_geopose_basic_quat(
    record: &GeoposeBasicQuatRecord,
) -> Result<DecodedGeopose, GeoposeError> {
    if record._pad != 0 {
        return Err(GeoposeError::NonZeroPadding);
    }
    Ok(DecodedGeopose {
        lat: record.lat,
        lon: record.lon,
        h: record.h,
        qx: record.qx,
        qy: record.qy,
        qz: record.qz,
        qw: record.qw,
    })
}
