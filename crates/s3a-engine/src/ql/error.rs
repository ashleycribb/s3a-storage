use std::fmt;
use std::io;

/// Structured error codes and details returned by the S3A-QL query engine.
#[derive(Debug)]
pub enum QlError {
    /// Syntax or lexical analysis error with source position.
    Parse {
        message: String,
        line: u32,
        col: u32,
    },
    /// A domain pack is required to execute this statement but is not registered or linked.
    PackRequired {
        pack: &'static str,
        feature: &'static str,
    },
    /// The archive header addon_mask does not have the required capability bit enabled.
    AddonRequired {
        addon_mask: u32,
        name: &'static str,
    },
    /// The tile layout version is incompatible with current reader capabilities.
    UnsupportedLayout {
        tile_type: u32,
        version: u32,
    },
    /// Direct coordinate fetch did not find a record.
    NotFound {
        coordinate: String,
    },
    /// Underlying filesystem or memory mapping I/O error.
    Io(io::Error),
    /// Internal execution or engine failure.
    Internal(String),
}

impl QlError {
    /// Returns the stable string error code for machine-readable APIs / CLI JSON output.
    pub fn code(&self) -> &'static str {
        match self {
            QlError::Parse { .. } => "parse_error",
            QlError::PackRequired { .. } => "pack_required",
            QlError::AddonRequired { .. } => "addon_required",
            QlError::UnsupportedLayout { .. } => "unsupported_layout",
            QlError::NotFound { .. } => "not_found",
            QlError::Io(_) => "io_error",
            QlError::Internal(_) => "internal_error",
        }
    }

    /// Serializes the error into a stable, machine-readable JSON string.
    pub fn to_json(&self) -> String {
        match self {
            QlError::Parse { message, line, col } => {
                format!(
                    r#"{{"ok":false,"code":"parse_error","message":"{}","line":{},"col":{}}}"#,
                    escape_json(message),
                    line,
                    col
                )
            }
            QlError::PackRequired { pack, feature } => {
                format!(
                    r#"{{"ok":false,"code":"pack_required","message":"Domain query requires pack '{}' for feature '{}'","pack":"{}","feature":"{}"}}"#,
                    pack, feature, pack, feature
                )
            }
            QlError::AddonRequired { addon_mask, name } => {
                format!(
                    r#"{{"ok":false,"code":"addon_required","message":"Archive does not have capability 0x{:04x} ('{}') enabled","addon_mask":"0x{:04x}","name":"{}"}}"#,
                    addon_mask, name, addon_mask, name
                )
            }
            QlError::UnsupportedLayout { tile_type, version } => {
                format!(
                    r#"{{"ok":false,"code":"unsupported_layout","message":"Unsupported record layout version {} for tile type {}","tile_type":{},"version":{}}}"#,
                    version, tile_type, tile_type, version
                )
            }
            QlError::NotFound { coordinate } => {
                format!(
                    r#"{{"ok":false,"code":"not_found","message":"Record not found at coordinate {}","coordinate":"{}"}}"#,
                    coordinate, coordinate
                )
            }
            QlError::Io(err) => {
                format!(
                    r#"{{"ok":false,"code":"io_error","message":"{}"}}"#,
                    escape_json(&err.to_string())
                )
            }
            QlError::Internal(msg) => {
                format!(
                    r#"{{"ok":false,"code":"internal_error","message":"{}"}}"#,
                    escape_json(msg)
                )
            }
        }
    }
}

impl fmt::Display for QlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QlError::Parse { message, line, col } => {
                write!(f, "Parse error at line {}, col {}: {}", line, col, message)
            }
            QlError::PackRequired { pack, feature } => {
                write!(f, "Pack required: feature '{}' requires domain pack '{}'", feature, pack)
            }
            QlError::AddonRequired { addon_mask, name } => {
                write!(f, "Capability required: archive requires addon bit 0x{:04x} ('{}')", addon_mask, name)
            }
            QlError::UnsupportedLayout { tile_type, version } => {
                write!(f, "Unsupported layout: tile type {} layout version {} is not supported", tile_type, version)
            }
            QlError::NotFound { coordinate } => {
                write!(f, "Record not found at coordinate '{}'", coordinate)
            }
            QlError::Io(err) => {
                write!(f, "I/O error: {}", err)
            }
            QlError::Internal(msg) => {
                write!(f, "Internal error: {}", msg)
            }
        }
    }
}

impl std::error::Error for QlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            QlError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for QlError {
    fn from(err: io::Error) -> Self {
        QlError::Io(err)
    }
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
