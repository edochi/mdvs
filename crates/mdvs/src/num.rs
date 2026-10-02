// --- Exact integer-to-float conversion ---

/// Largest magnitude at or below which f64 represents every integer exactly (2^53).
pub(crate) const F64_EXACT_INT_LIMIT: u64 = 1 << 53;

/// 2^32 as an f64, used to scale the high half of a split integer.
const TWO_POW_32: f64 = 4_294_967_296.0;

/// Number of bits in the low half of a split integer.
const LOW_HALF_BITS: u32 = 32;

/// Mask selecting the low 32 bits of an i64.
const LOW_HALF_MASK: i64 = 0xFFFF_FFFF;

/// Exact i64 → f64; None when |i| > 2^53, where f64 can no longer represent every integer.
///
/// The integer is split as `i = hi * 2^32 + lo` with `hi = i >> 32`
/// (arithmetic shift, so it carries the sign) and `lo = i & 0xFFFF_FFFF`
/// (always in `0..2^32`). Within the guard, `hi` fits in i32 and `lo` in u32,
/// so both convert to f64 through lossless `From`. Multiplying by 2^32 only
/// changes the exponent, so `hi * 2^32` is exact. The final addition has two
/// exact operands whose true sum `i` is representable (`|i| <= 2^53`), and
/// IEEE 754 addition is correctly rounded, so the result is exactly `i`.
pub(crate) fn i64_to_f64_exact(i: i64) -> Option<f64> {
    if i.unsigned_abs() > F64_EXACT_INT_LIMIT {
        return None;
    }
    let hi = f64::from(i32::try_from(i >> LOW_HALF_BITS).ok()?);
    let lo = f64::from(u32::try_from(i & LOW_HALF_MASK).ok()?);
    Some(hi * TWO_POW_32 + lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2^53 as an f64 literal.
    const TWO_POW_53: f64 = 9_007_199_254_740_992.0;

    /// -(2^32 + 5): a negative value with both halves non-trivial.
    const NEG_SPLIT: i64 = -4_294_967_301;

    fn limit() -> i64 {
        i64::try_from(F64_EXACT_INT_LIMIT).unwrap()
    }

    #[test]
    fn zero_converts_exactly() {
        assert_eq!(i64_to_f64_exact(0), Some(0.0));
    }

    #[test]
    fn minus_one_converts_exactly() {
        assert_eq!(i64_to_f64_exact(-1), Some(-1.0));
    }

    #[test]
    fn negative_value_spanning_both_halves_converts_exactly() {
        assert_eq!(i64_to_f64_exact(NEG_SPLIT), Some(-4_294_967_301.0));
    }

    #[test]
    fn positive_limit_converts_exactly() {
        assert_eq!(i64_to_f64_exact(limit()), Some(TWO_POW_53));
    }

    #[test]
    fn negative_limit_converts_exactly() {
        assert_eq!(i64_to_f64_exact(-limit()), Some(-TWO_POW_53));
    }

    #[test]
    fn just_below_limit_converts_exactly() {
        assert_eq!(i64_to_f64_exact(limit() - 1), Some(9_007_199_254_740_991.0));
    }

    #[test]
    fn positive_beyond_limit_is_none() {
        assert_eq!(i64_to_f64_exact(limit() + 1), None);
    }

    #[test]
    fn negative_beyond_limit_is_none() {
        assert_eq!(i64_to_f64_exact(-limit() - 1), None);
    }

    #[test]
    fn i64_extremes_are_none() {
        assert_eq!(i64_to_f64_exact(i64::MAX), None);
        assert_eq!(i64_to_f64_exact(i64::MIN), None);
    }
}
