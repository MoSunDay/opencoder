//! Consumer identity and cursor values for durable event replay.
//!
//! A consumer id names one durable replay position in the event log (see
//! [`crate::ledger::EventLedger`]). The id becomes part of a storage key
//! in every backend (`evc/<consumer-id>` in the rdb layout), so it is
//! validated as strict pure logic shared by all implementations: non-empty,
//! bounded length, ascii, no whitespace and no `/` (the key separator).
//! Callers must never trust an arbitrary free-form string as a key.

/// Upper bound of a consumer id, in bytes.
pub const MAX_CONSUMER_ID_BYTES: usize = 128;

/// Lowest meaningful replay cursor: log seqs are strictly positive, so 0
/// means "nothing consumed yet".
pub const START_CURSOR: i64 = 0;

/// Validation failures for a consumer id or cursor value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConsumerError {
    #[error("consumer id must be non-empty ascii without whitespace or '/'")]
    InvalidId,
    #[error("consumer id must be at most {MAX_CONSUMER_ID_BYTES} bytes")]
    IdTooLong,
    #[error("consumer cursor must be >= {START_CURSOR}")]
    CursorTooLow,
}

/// Validates a consumer id: non-empty, ascii, no whitespace, no `/`,
/// at most [`MAX_CONSUMER_ID_BYTES`] bytes.
pub fn validate_consumer_id(id: &str) -> Result<(), ConsumerError> {
    let shape_ok = !id.is_empty()
        && id.is_ascii()
        && !id.bytes().any(|b| b == b'/')
        && !id.chars().any(char::is_whitespace);
    if !shape_ok {
        return Err(ConsumerError::InvalidId);
    }
    if id.len() > MAX_CONSUMER_ID_BYTES {
        return Err(ConsumerError::IdTooLong);
    }
    Ok(())
}

/// Validates a replay cursor: seqs below [`START_CURSOR`] are meaningless
/// (log seqs are positive) and must not be persisted.
pub fn validate_cursor(seq: i64) -> Result<(), ConsumerError> {
    if seq < START_CURSOR {
        Err(ConsumerError::CursorTooLow)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_ids_and_rejects_key_unsafe_ones() {
        assert_eq!(validate_consumer_id("pump"), Ok(()));
        assert_eq!(
            validate_consumer_id(&"a".repeat(MAX_CONSUMER_ID_BYTES)),
            Ok(())
        );

        for bad in ["", " ", "id with space", "tab\tid", "a/b", "ünicode", "\n"] {
            assert_eq!(
                validate_consumer_id(bad),
                Err(ConsumerError::InvalidId),
                "{bad:?}"
            );
        }
        let too_long = "a".repeat(MAX_CONSUMER_ID_BYTES + 1);
        assert_eq!(
            validate_consumer_id(&too_long),
            Err(ConsumerError::IdTooLong)
        );
    }

    #[test]
    fn cursor_must_be_non_negative() {
        assert_eq!(validate_cursor(0), Ok(()));
        assert_eq!(validate_cursor(17), Ok(()));
        assert_eq!(validate_cursor(-1), Err(ConsumerError::CursorTooLow));
    }
}
