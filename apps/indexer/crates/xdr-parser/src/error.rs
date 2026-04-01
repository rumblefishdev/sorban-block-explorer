//! Error types for the XDR parser.
//!
//! Design principle: never drop data. Malformed XDR produces a partial record
//! with `parse_error = true`, not a skipped transaction.

use std::fmt;

/// Context about where an error occurred, for structured logging.
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub ledger_sequence: Option<u32>,
    pub transaction_index: Option<usize>,
    pub transaction_hash: Option<String>,
    pub field: Option<String>,
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(seq) = self.ledger_sequence {
            parts.push(format!("ledger={seq}"));
        }
        if let Some(idx) = self.transaction_index {
            parts.push(format!("tx_index={idx}"));
        }
        if let Some(ref hash) = self.transaction_hash {
            parts.push(format!("tx_hash={}", &hash[..16.min(hash.len())]));
        }
        if let Some(ref field) = self.field {
            parts.push(format!("field={field}"));
        }
        write!(f, "[{}]", parts.join(", "))
    }
}

/// Kinds of parsing errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    DecompressionFailed,
    XdrDeserializationFailed,
    InvalidS3Key,
    XdrSerializationFailed,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecompressionFailed => write!(f, "DECOMPRESSION_FAILED"),
            Self::XdrDeserializationFailed => write!(f, "XDR_DESERIALIZATION_FAILED"),
            Self::InvalidS3Key => write!(f, "INVALID_S3_KEY"),
            Self::XdrSerializationFailed => write!(f, "XDR_SERIALIZATION_FAILED"),
        }
    }
}

/// A structured parse error with context for logging and partial-record handling.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub message: String,
    pub context: Option<ErrorContext>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)?;
        if let Some(ref ctx) = self.context {
            write!(f, " {ctx}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    pub fn with_context(mut self, ctx: ErrorContext) -> Self {
        self.context = Some(ctx);
        self
    }
}
