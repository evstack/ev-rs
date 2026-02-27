use evolve_core::{ErrorCode, define_error};
use fixed::{FixedU128, types::extra::U64};
use std::ops::{Add, Div, Mul, Sub};

define_error!(ERR_DIVISION_BY_ZERO, 0x1, "divide by zero");

/// A UQ64.64 fixed-point number representation
///
/// UQ64.64 is an unsigned fixed-point number with 64 integer bits and 64 fractional bits
/// This is a wrapper around `FixedU128<U64>` from the fixed crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UQ64x64(pub FixedU128<U64>);

impl UQ64x64 {
    /// Constant for zero
    pub const ZERO: Self = Self(FixedU128::<U64>::from_bits(0));

    /// Constant for one
    pub const ONE: Self = Self(FixedU128::<U64>::from_bits(1 << 64));

    /// Constant for two
    pub const TWO: Self = Self(FixedU128::<U64>::from_bits(2 << 64));

    /// Constant for three
    pub const THREE: Self = Self(FixedU128::<U64>::from_bits(3 << 64));

    /// Constant for four
    pub const FOUR: Self = Self(FixedU128::<U64>::from_bits(4 << 64));

    /// Constant for five
    pub const FIVE: Self = Self(FixedU128::<U64>::from_bits(5 << 64));

    /// Constant for ten
    pub const TEN: Self = Self(FixedU128::<U64>::from_bits(10 << 64));

    /// Constant for one hundred
    pub const HUNDRED: Self = Self(FixedU128::<U64>::from_bits(100 << 64));

    /// Constant for one thousand
    pub const THOUSAND: Self = Self(FixedU128::<U64>::from_bits(1000 << 64));

    /// Create a new UQ64.64 fixed-point number with the raw value
    pub const fn new(raw_value: u128) -> Self {
        Self(FixedU128::<U64>::from_bits(raw_value))
    }

    /// Create a UQ64.64 from a whole number (becomes N.0)
    pub fn from_u64(value: u64) -> Self {
        Self(FixedU128::<U64>::from_num(value))
    }

    /// Create a UQ64.64 from a u128 (becomes N.0)
    pub fn from_u128(value: u128) -> Self {
        Self(FixedU128::<U64>::from_num(value))
    }

    /// Create a UQ64.64 from a fraction (numerator/denominator)
    pub fn from_fraction(numerator: u64, denominator: u64) -> Result<Self, ErrorCode> {
        // Handle possible division by zero
        if denominator == 0 {
            return Err(ERR_DIVISION_BY_ZERO);
        }

        let num = FixedU128::<U64>::from_num(numerator);
        let den = FixedU128::<U64>::from_num(denominator);

        Ok(Self(num / den))
    }

    /// Get the integer part of the fixed-point number
    pub fn integer_part(&self) -> u64 {
        self.0.int().to_num()
    }

    /// Get the fractional part as a raw u64 value
    /// This is the fractional part multiplied by 2^64
    pub fn frac_raw(&self) -> u64 {
        self.0.frac().to_bits() as u64
    }

    /// Get the raw bits of the fixed-point number
    pub fn to_bits(&self) -> u128 {
        self.0.to_bits()
    }

    /// Safely divide this value by another, returning an error on division by zero
    pub fn checked_div(&self, rhs: Self) -> Result<Self, ErrorCode> {
        if rhs.0 == FixedU128::<U64>::from_num(0) {
            return Err(ERR_DIVISION_BY_ZERO);
        }

        Ok(Self(self.0 / rhs.0))
    }

    /// Safely divide a u128 by this fixed-point number, returning an error on division by zero
    pub fn checked_div_from_u128(value: u128, divisor: Self) -> Result<Self, ErrorCode> {
        if divisor.0 == FixedU128::<U64>::from_num(0) {
            return Err(ERR_DIVISION_BY_ZERO);
        }

        Ok(Self(FixedU128::<U64>::from_num(value) / divisor.0))
    }

    /// Create a UQ64.64 from a percentage value (divides by 100)
    /// For example, percentage(50) creates 0.5 (50%)
    pub fn percentage(value: u64) -> Self {
        let num = FixedU128::<U64>::from_num(value);
        let den = FixedU128::<U64>::from_num(100u64);
        Self(num / den)
    }

    /// Create a UQ64.64 from basis points (divides by 10000)
    /// For example, bps(50) creates 0.005 (0.5%)
    pub fn bps(value: u64) -> Self {
        let num = FixedU128::<U64>::from_num(value);
        let den = FixedU128::<U64>::from_num(10_000u64);
        Self(num / den)
    }
}

// Implement Add operation
impl Add for UQ64x64 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

// Implement Sub operation
impl Sub for UQ64x64 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

// Implement Mul operation
impl Mul for UQ64x64 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

// Implement Div operation
impl Div for UQ64x64 {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        Self(self.0 / rhs.0)
    }
}

// From<u64> implementation for convenient construction
impl From<u64> for UQ64x64 {
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

// From<u128> implementation
impl From<u128> for UQ64x64 {
    fn from(value: u128) -> Self {
        Self::from_u128(value)
    }
}

// Implement multiplication with u128
impl Mul<u128> for UQ64x64 {
    type Output = Self;

    fn mul(self, rhs: u128) -> Self::Output {
        Self(self.0 * FixedU128::<U64>::from_num(rhs))
    }
}

// Implement right-side multiplication (u128 * UQ64x64)
impl Mul<UQ64x64> for u128 {
    type Output = UQ64x64;

    fn mul(self, rhs: UQ64x64) -> UQ64x64 {
        UQ64x64(FixedU128::<U64>::from_num(self) * rhs.0)
    }
}

// Implement division by u128
impl Div<u128> for UQ64x64 {
    type Output = Self;

    fn div(self, rhs: u128) -> Self::Output {
        if rhs == 0 {
            panic!("Division by zero");
        }
        Self(self.0 / FixedU128::<U64>::from_num(rhs))
    }
}

// Implement checked division by u128 that returns a Result
impl UQ64x64 {
    /// Safely divide by a u128, returning an error on division by zero
    pub fn checked_div_u128(&self, rhs: u128) -> Result<Self, ErrorCode> {
        if rhs == 0 {
            return Err(ERR_DIVISION_BY_ZERO);
        }

        Ok(Self(self.0 / FixedU128::<U64>::from_num(rhs)))
    }
}

// Implement division of u128 by UQ64x64
impl Div<UQ64x64> for u128 {
    type Output = UQ64x64;

    fn div(self, rhs: UQ64x64) -> UQ64x64 {
        if rhs.0 == FixedU128::<U64>::from_num(0) {
            panic!("Division by zero");
        }
        UQ64x64(FixedU128::<U64>::from_num(self) / rhs.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_u64() {
        let one = UQ64x64::from_u64(1);
        let two = UQ64x64::from_u64(2);

        assert_eq!(one.integer_part(), 1);
        assert_eq!(two.integer_part(), 2);
    }

    #[test]
    fn test_from_fraction() {
        let half = UQ64x64::from_fraction(1, 2).unwrap();
        let quarter = UQ64x64::from_fraction(1, 4).unwrap();

        assert_eq!(half.integer_part(), 0);
        assert_eq!(quarter.integer_part(), 0);

        let one_and_half = UQ64x64::from_fraction(3, 2).unwrap();
        assert_eq!(one_and_half.integer_part(), 1);
    }

    #[test]
    fn test_division_by_zero() {
        let result = UQ64x64::from_fraction(1, 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_DIVISION_BY_ZERO);
    }

    #[test]
    fn test_checked_div() {
        let one = UQ64x64::from_u64(1);
        let two = UQ64x64::from_u64(2);
        let zero = UQ64x64::from_u64(0);

        // Normal division should work
        let half = one.checked_div(two).unwrap();
        assert_eq!(half.integer_part(), 0);

        // Division by zero should return an error
        let result = one.checked_div(zero);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_DIVISION_BY_ZERO);
    }

    #[test]
    fn test_u128_operations() {
        let fixed = UQ64x64::from_u64(10);
        let u128_val = 5u128;

        // Test multiplication UQ64x64 * u128
        let mul_result = fixed * u128_val;
        assert_eq!(mul_result.integer_part(), 50);

        // Test multiplication u128 * UQ64x64
        let mul_result2 = u128_val * fixed;
        assert_eq!(mul_result2.integer_part(), 50);

        // Test division UQ64x64 / u128
        let div_result = fixed / u128_val;
        assert_eq!(div_result.integer_part(), 2);

        // Test division u128 / UQ64x64
        let div_result2 = u128_val / fixed;
        assert_eq!(div_result2.integer_part(), 0); // 5/10 = 0.5

        // Test checked division UQ64x64 / u128
        let checked_div = fixed.checked_div_u128(u128_val).unwrap();
        assert_eq!(checked_div.integer_part(), 2);

        // Test checked division u128 / UQ64x64
        let checked_div2 = UQ64x64::checked_div_from_u128(u128_val, fixed).unwrap();
        assert_eq!(checked_div2.integer_part(), 0); // 5/10 = 0.5

        // Test division by zero
        let div_by_zero = fixed.checked_div_u128(0);
        assert!(div_by_zero.is_err());
        assert_eq!(div_by_zero.unwrap_err(), ERR_DIVISION_BY_ZERO);

        // Test division by zero (u128 / UQ64x64)
        let zero_fixed = UQ64x64::from_u64(0);
        let div_by_zero2 = UQ64x64::checked_div_from_u128(u128_val, zero_fixed);
        assert!(div_by_zero2.is_err());
        assert_eq!(div_by_zero2.unwrap_err(), ERR_DIVISION_BY_ZERO);
    }

    #[test]
    fn test_operations() {
        let one = UQ64x64::from_u64(1);
        let two = UQ64x64::from_u64(2);
        let half = UQ64x64::from_fraction(1, 2).unwrap();

        // Test addition
        let three = one + two;
        assert_eq!(three.integer_part(), 3);

        // Test subtraction
        let one_again = three - two;
        assert_eq!(one_again.integer_part(), 1);

        // Test multiplication
        let half_again = one * half;
        assert_eq!(half_again.integer_part(), 0);

        // Test division
        let two_again = one / half;
        assert_eq!(two_again.integer_part(), 2);
    }

    #[test]
    fn test_constants() {
        assert_eq!(UQ64x64::ZERO.integer_part(), 0);
        assert_eq!(UQ64x64::ONE.integer_part(), 1);
        assert_eq!(UQ64x64::TWO.integer_part(), 2);
        assert_eq!(UQ64x64::THREE.integer_part(), 3);
        assert_eq!(UQ64x64::FOUR.integer_part(), 4);
        assert_eq!(UQ64x64::FIVE.integer_part(), 5);
        assert_eq!(UQ64x64::TEN.integer_part(), 10);
        assert_eq!(UQ64x64::HUNDRED.integer_part(), 100);
        assert_eq!(UQ64x64::THOUSAND.integer_part(), 1000);
    }

    #[test]
    fn test_percentage() {
        // 100% = 1.0
        let one_hundred_percent = UQ64x64::percentage(100);
        assert_eq!(one_hundred_percent.integer_part(), 1);
        assert_eq!(one_hundred_percent, UQ64x64::ONE);

        // 50% = 0.5
        let fifty_percent = UQ64x64::percentage(50);
        assert_eq!(fifty_percent.integer_part(), 0);
        assert_eq!(fifty_percent, UQ64x64::from_fraction(1, 2).unwrap());

        // 10% = 0.1
        let ten_percent = UQ64x64::percentage(10);
        assert_eq!(ten_percent.integer_part(), 0);
        assert_eq!(ten_percent, UQ64x64::from_fraction(1, 10).unwrap());
    }

    #[test]
    fn test_basis_points() {
        // 10000 bps = 100% = 1.0
        let ten_thousand_bps = UQ64x64::bps(10000);
        assert_eq!(ten_thousand_bps.integer_part(), 1);
        assert_eq!(ten_thousand_bps, UQ64x64::ONE);

        // 5000 bps = 50% = 0.5
        let five_thousand_bps = UQ64x64::bps(5000);
        assert_eq!(five_thousand_bps.integer_part(), 0);
        assert_eq!(five_thousand_bps, UQ64x64::from_fraction(1, 2).unwrap());

        // 100 bps = 1% = 0.01
        let one_hundred_bps = UQ64x64::bps(100);
        assert_eq!(one_hundred_bps.integer_part(), 0);
        assert_eq!(one_hundred_bps, UQ64x64::from_fraction(1, 100).unwrap());

        // 1 bps = 0.01% = 0.0001
        let one_bps = UQ64x64::bps(1);
        assert_eq!(one_bps.integer_part(), 0);
        assert_eq!(one_bps, UQ64x64::from_fraction(1, 10000).unwrap());
    }
}
