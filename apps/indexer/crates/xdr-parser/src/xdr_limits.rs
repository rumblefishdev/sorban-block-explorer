//! XDR deserialization/serialization limits configuration.

use stellar_xdr::curr::Limits;

/// Default XDR deserialization limits, sized to the input payload.
///
/// `depth` = 1000 handles deeply nested Soroban invocation trees.
/// `len` = max(input_size, 10MB) ensures we can deserialize large ledgers.
pub fn deserialization_limits(input_len: usize) -> Limits {
    Limits {
        depth: 1000,
        len: input_len.max(10_000_000),
    }
}

/// Default XDR serialization limits for re-encoding values (e.g., for base64 retention).
///
/// Uses a generous 10MB bound since we're serializing individual objects,
/// not full batches.
pub fn serialization_limits() -> Limits {
    Limits {
        depth: 1000,
        len: 10_000_000,
    }
}
