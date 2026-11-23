/// Errors returned by the CAN bridge. Every path fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanError {
    MissingSourceId,
    MissingOriginPtr,
    TruncatedRecord { expected: usize, actual: usize },
    UnsupportedLayoutVersion { tile_type: u32, version: u32 },
    UnknownTileType(u32),
    ZeroFaultNodeId,
    MalformedLog { line: usize },
    IncompleteBam,
    UnexpectedTpDt,
    BamSequence,
    ShortPayload,
    InvalidObdResponse,
    UnknownMappingKind,
    DuplicateMappingKey,
    MalformedMapping { line: usize },
    MalformedThreshold { line: usize },
    NotJ1939Id { can_id: u32 },
    SocketCanOpenFailed,
    NonZeroPad,
    UnknownDtcKind { kind: u8 },
    UnknownTriggerKind { kind: u8 },
    UnsupportedTpCm { control: u8 },
}

/// Result of a mapping-table lookup. Unmapped values are `NoMatch`; the
/// bridge never invents a node id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerResult {
    Match { fault_node_id: u64 },
    NoMatch,
}
