//! Backward-compatible scheduling policies; pure decisions shared by runtime and API.

/// Hard upper bound for `DagSpec.max_concurrency` (whole-run parallelism).
pub const MAX_CONCURRENCY: usize = 30;

/// Default whole-run parallelism when a spec omits `max_concurrency`.
pub fn default_concurrency() -> usize {
    4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_exceeds_the_default() {
        assert!(MAX_CONCURRENCY >= default_concurrency());
        assert_eq!(default_concurrency(), 4);
    }
}
