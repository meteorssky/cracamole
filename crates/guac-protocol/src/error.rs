use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("non-digit character in element length")]
    BadLength,
    #[error("element length exceeds maximum")]
    ElementTooLong,
    #[error("too many elements in instruction")]
    TooManyElements,
    #[error("element not followed by ',' or ';'")]
    BadTerminator,
    #[error("element is not valid UTF-8")]
    InvalidUtf8,
    #[error("parser previously failed")]
    Poisoned,
}
