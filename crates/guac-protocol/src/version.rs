use std::fmt;

/// Protocol versions (values from C `guac_protocol_version`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtocolVersion {
    V1_0_0,
    V1_1_0,
    V1_3_0,
    V1_5_0,
}

impl ProtocolVersion {
    pub const LATEST: Self = Self::V1_5_0;

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "VERSION_1_0_0" => Self::V1_0_0,
            "VERSION_1_1_0" => Self::V1_1_0,
            "VERSION_1_3_0" => Self::V1_3_0,
            "VERSION_1_5_0" => Self::V1_5_0,
            _ => return None,
        })
    }

    /// Version both sides can speak.
    pub fn negotiate(self, other: Self) -> Self {
        self.min(other)
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::V1_0_0 => "VERSION_1_0_0",
            Self::V1_1_0 => "VERSION_1_1_0",
            Self::V1_3_0 => "VERSION_1_3_0",
            Self::V1_5_0 => "VERSION_1_5_0",
        })
    }
}
