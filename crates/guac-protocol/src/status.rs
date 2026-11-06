/// `error` instruction status codes (C `guac_protocol_status`, protocol-types.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Status {
    Success = 0x0000,
    Unsupported = 0x0100,
    ServerError = 0x0200,
    ServerBusy = 0x0201,
    UpstreamTimeout = 0x0202,
    UpstreamError = 0x0203,
    ResourceNotFound = 0x0204,
    ResourceConflict = 0x0205,
    ResourceClosed = 0x0206,
    UpstreamNotFound = 0x0207,
    UpstreamUnavailable = 0x0208,
    SessionConflict = 0x0209,
    SessionTimeout = 0x020A,
    SessionClosed = 0x020B,
    ClientBadRequest = 0x0300,
    ClientUnauthorized = 0x0301,
    ClientForbidden = 0x0303,
    ClientTimeout = 0x0308,
    ClientOverrun = 0x030D,
    ClientBadType = 0x030F,
    ClientTooMany = 0x031D,
}

impl Status {
    pub const ALL: [Status; 21] = [
        Status::Success,
        Status::Unsupported,
        Status::ServerError,
        Status::ServerBusy,
        Status::UpstreamTimeout,
        Status::UpstreamError,
        Status::ResourceNotFound,
        Status::ResourceConflict,
        Status::ResourceClosed,
        Status::UpstreamNotFound,
        Status::UpstreamUnavailable,
        Status::SessionConflict,
        Status::SessionTimeout,
        Status::SessionClosed,
        Status::ClientBadRequest,
        Status::ClientUnauthorized,
        Status::ClientForbidden,
        Status::ClientTimeout,
        Status::ClientOverrun,
        Status::ClientBadType,
        Status::ClientTooMany,
    ];

    pub fn code(self) -> u16 {
        self as u16
    }

    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.code() == code)
    }
}
