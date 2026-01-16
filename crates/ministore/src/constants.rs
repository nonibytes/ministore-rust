// Guardrails and configuration constants from DESIGN.md

/// Minimum length for contains wildcard inner pattern (e.g., *foo* requires len(foo) >= 3)
pub const MIN_CONTAINS_LEN: usize = 3;

/// Minimum literal prefix length for prefix wildcards (e.g., foo* requires len(foo) >= 2)
pub const MIN_PREFIX_LEN: usize = 2;

/// Maximum number of dictionary entries to expand for prefix patterns
pub const MAX_PREFIX_EXPANSION: usize = 20_000;

/// Default cursor TTL in milliseconds (1 hour)
pub const CURSOR_TTL_MS: i64 = 60 * 60 * 1000;
