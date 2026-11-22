//! Exact rationals for compile-time arithmetic, sub-bar timing and tempo maths (docs/BTW-EXTENSIONS.md).
//!
//! A [`Rat`] is `num / den` in lowest terms with `den > 0`. Every operation is checked: an overflow is an
//! `Err`, never a wrap or a panic, so a program's arithmetic is total. Printing is canonical (`3`, `1/2`,
//! `-5/4`), which is what the canonical text of a program uses.

use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rat {
    num: i64,
    den: i64,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// What a failed operation says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RatError {
    DivideByZero,
    Overflow,
}

impl fmt::Display for RatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RatError::DivideByZero => write!(f, "division by zero"),
            RatError::Overflow => write!(f, "the number is too large"),
        }
    }
}

type R = Result<Rat, RatError>;

impl Rat {
    pub const ZERO: Rat = Rat { num: 0, den: 1 };
    pub const ONE: Rat = Rat { num: 1, den: 1 };

    pub fn int(n: i64) -> Rat {
        Rat { num: n, den: 1 }
    }

    /// `n / d` in lowest terms.
    pub fn new(n: i64, d: i64) -> R {
        Rat::from_i128(i128::from(n), i128::from(d))
    }

    fn from_i128(n: i128, d: i128) -> R {
        if d == 0 {
            return Err(RatError::DivideByZero);
        }
        let g = gcd(n, d).max(1);
        let (mut n, mut d) = (n / g, d / g);
        if d < 0 {
            n = -n;
            d = -d;
        }
        match (i64::try_from(n), i64::try_from(d)) {
            (Ok(num), Ok(den)) => Ok(Rat { num, den }),
            _ => Err(RatError::Overflow),
        }
    }

    pub fn numer(self) -> i64 {
        self.num
    }

    pub fn denom(self) -> i64 {
        self.den
    }

    pub fn is_int(self) -> bool {
        self.den == 1
    }

    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    pub fn is_neg(self) -> bool {
        self.num < 0
    }

    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// The exact decimal this rational is when it has one of at most six places (its denominator divides 10^6): `7/10` is
    /// `0.7`, `-3/2` is `-1.5`, `5` is `5`. `None` for `1/3` and for `1/10000000`: the canonical text prints those as the
    /// fraction, so that what it says is exactly what was meant (a rounded decimal would be another number).
    pub fn decimal6(self) -> Option<String> {
        const K: i128 = 1_000_000;
        let scaled = i128::from(self.num) * K;
        let den = i128::from(self.den);
        if scaled % den != 0 {
            return None;
        }
        let k = scaled / den;
        let (whole, frac) = (k.abs() / K, k.abs() % K);
        let mut s = if frac == 0 { whole.to_string() } else { format!("{whole}.{frac:06}").trim_end_matches('0').to_string() };
        if k < 0 {
            s.insert(0, '-');
        }
        Some(s)
    }

    /// The nearest rational to `v` whose denominator is at most 1,000,000 (the way a decimal such as
    /// `0.125` or `0.3` that was typed comes back exactly: `from_f64(1.0 / 3.0)` is `1/3`).
    pub fn from_f64(v: f64) -> Option<Rat> {
        if !v.is_finite() || v.abs() > 1e15 {
            return None;
        }
        let neg = v < 0.0;
        let x = v.abs();
        // a decimal of at most six places, as typed, is exactly that: `0.999999` is 999999/1000000, not the shorter fraction
        // (999998/999999) that is within 1e-12 of it, which the canonical text would then print as a fraction
        let k = (x * 1e6).round();
        if (x - k / 1e6).abs() <= 1e-12 * x.max(1.0) {
            let n = k as i128;
            return Rat::from_i128(if neg { -n } else { n }, 1_000_000).ok();
        }
        // continued fraction, stopped when the denominator would pass the limit or the error is below 1e-12
        let (mut h0, mut h1, mut k0, mut k1): (i128, i128, i128, i128) = (0, 1, 1, 0);
        let mut r = x;
        for _ in 0..64 {
            let a = r.floor();
            let ai = a as i128;
            let (h2, k2) = (ai * h1 + h0, ai * k1 + k0);
            if k2 > 1_000_000 {
                break;
            }
            (h0, h1, k0, k1) = (h1, h2, k1, k2);
            if (x - h1 as f64 / k1 as f64).abs() < 1e-12 * x.max(1.0) {
                break;
            }
            let frac = r - a;
            if frac < 1e-15 {
                break;
            }
            r = 1.0 / frac;
        }
        let n = if neg { -h1 } else { h1 };
        Rat::from_i128(n, k1).ok()
    }

    pub fn add(self, o: Rat) -> R {
        Rat::from_i128(i128::from(self.num) * i128::from(o.den) + i128::from(o.num) * i128::from(self.den), i128::from(self.den) * i128::from(o.den))
    }

    pub fn sub(self, o: Rat) -> R {
        Rat::from_i128(i128::from(self.num) * i128::from(o.den) - i128::from(o.num) * i128::from(self.den), i128::from(self.den) * i128::from(o.den))
    }

    pub fn mul(self, o: Rat) -> R {
        Rat::from_i128(i128::from(self.num) * i128::from(o.num), i128::from(self.den) * i128::from(o.den))
    }

    pub fn div(self, o: Rat) -> R {
        if o.num == 0 {
            return Err(RatError::DivideByZero);
        }
        Rat::from_i128(i128::from(self.num) * i128::from(o.den), i128::from(self.den) * i128::from(o.num))
    }

    pub fn neg(self) -> R {
        Rat::from_i128(-i128::from(self.num), i128::from(self.den))
    }

    pub fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    pub fn ceil(self) -> i64 {
        -((-self.num).div_euclid(self.den))
    }

    /// `self - o * floor(self / o)`: the modulo of a Euclidean division, always `0 <= r < |o|` for `o > 0`.
    pub fn rem(self, o: Rat) -> R {
        if o.num == 0 {
            return Err(RatError::DivideByZero);
        }
        let q = self.div(o)?;
        let f = Rat::int(q.floor());
        self.sub(o.mul(f)?)
    }

    pub fn min(self, o: Rat) -> Rat {
        if o < self {
            o
        } else {
            self
        }
    }

    pub fn max(self, o: Rat) -> Rat {
        if o > self {
            o
        } else {
            self
        }
    }

    /// The whole number it is, if it is one.
    pub fn as_int(self) -> Option<i64> {
        self.is_int().then_some(self.num)
    }

    /// `n` if it is a whole number in `0..=u32::MAX`.
    pub fn as_u32(self) -> Option<u32> {
        self.as_int().and_then(|n| u32::try_from(n).ok())
    }
}

impl PartialOrd for Rat {
    fn partial_cmp(&self, o: &Rat) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Rat {
    fn cmp(&self, o: &Rat) -> Ordering {
        (i128::from(self.num) * i128::from(o.den)).cmp(&(i128::from(o.num) * i128::from(self.den)))
    }
}

impl fmt::Display for Rat {
    /// Canonical: `3`, `1/2`, `-5/4`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> Rat {
        Rat::new(n, d).unwrap()
    }

    #[test]
    fn lowest_terms_and_sign() {
        assert_eq!(r(2, 4), r(1, 2));
        assert_eq!(r(1, -2), r(-1, 2));
        assert_eq!(r(0, 5), Rat::ZERO);
        assert_eq!(r(6, 3).to_string(), "2");
        assert_eq!(r(-10, 4).to_string(), "-5/2");
        assert_eq!(Rat::new(1, 0), Err(RatError::DivideByZero));
    }

    #[test]
    fn arithmetic_is_exact() {
        assert_eq!(r(1, 3).add(r(1, 6)).unwrap(), r(1, 2));
        assert_eq!(r(1, 2).sub(r(3, 4)).unwrap(), r(-1, 4));
        assert_eq!(r(2, 3).mul(r(3, 4)).unwrap(), r(1, 2));
        assert_eq!(r(1, 2).div(r(1, 4)).unwrap(), Rat::int(2));
        assert_eq!(r(7, 2).rem(Rat::int(2)).unwrap(), r(3, 2));
        assert_eq!(r(-1, 2).rem(Rat::int(2)).unwrap(), r(3, 2));
        assert_eq!(Rat::int(7).rem(Rat::int(4)).unwrap(), Rat::int(3));
        assert_eq!(Rat::int(1).div(Rat::ZERO), Err(RatError::DivideByZero));
    }

    #[test]
    fn overflow_is_an_error_not_a_wrap() {
        let big = Rat::int(i64::MAX);
        assert_eq!(big.add(big), Err(RatError::Overflow));
        assert_eq!(big.mul(big), Err(RatError::Overflow));
        assert!(big.sub(big).unwrap().is_zero());
    }

    #[test]
    fn ordering_and_floor() {
        assert!(r(1, 3) < r(1, 2));
        assert!(r(-1, 2) < Rat::ZERO);
        assert_eq!(r(7, 2).floor(), 3);
        assert_eq!(r(-7, 2).floor(), -4);
        assert_eq!(r(7, 2).ceil(), 4);
        assert_eq!(r(-7, 2).ceil(), -3);
        assert_eq!(r(1, 2).min(r(1, 3)), r(1, 3));
        assert_eq!(r(1, 2).max(r(1, 3)), r(1, 2));
    }

    #[test]
    fn a_rational_with_a_short_decimal_prints_as_it_and_any_other_does_not() {
        let d = |n: i64, den: i64| r(n, den).decimal6();
        assert_eq!(d(1, 2).as_deref(), Some("0.5"));
        assert_eq!(d(7, 10).as_deref(), Some("0.7"));
        assert_eq!(d(-3, 2).as_deref(), Some("-1.5"));
        assert_eq!(d(5, 1).as_deref(), Some("5"));
        assert_eq!(d(0, 1).as_deref(), Some("0"));
        assert_eq!(d(1, 1_000_000).as_deref(), Some("0.000001"));
        assert_eq!(d(333_333, 1_000_000).as_deref(), Some("0.333333"));
        assert_eq!(d(-1, 64).as_deref(), Some("-0.015625"));
        assert_eq!(d(1, 3), None);
        assert_eq!(d(1, 7), None);
        assert_eq!(d(1, 64 * 2), None, "seven places");
        assert_eq!(d(1, 10_000_000), None);
        // every decimal of at most six places reads back as the same rational (what the canonical text relies on)
        for k in [1i64, 3, 7, 25, 125, 999, 1001, 12_345, 500_000, 999_999] {
            for places in 0..=6u32 {
                let den = 10i64.pow(places);
                let v = r(k, den);
                let text = v.decimal6().unwrap();
                assert_eq!(Rat::from_f64(text.parse::<f64>().unwrap()), Some(v), "{text}");
            }
        }
    }

    #[test]
    fn decimals_that_were_typed_come_back_exactly() {
        for (v, n, d) in [(0.5, 1, 2), (0.25, 1, 4), (0.3, 3, 10), (0.7, 7, 10), (0.125, 1, 8), (2.75, 11, 4), (1.0 / 3.0, 1, 3), (2.0 / 3.0, 2, 3), (7.0, 7, 1), (0.1, 1, 10), (-1.5, -3, 2)] {
            assert_eq!(Rat::from_f64(v), Some(r(n, d)), "{v}");
        }
        assert_eq!(Rat::from_f64(f64::NAN), None);
    }

    #[test]
    fn f64_round_trip_of_small_rationals() {
        for d in 1..=64i64 {
            for n in -64..=64i64 {
                let q = r(n, d);
                assert_eq!(Rat::from_f64(q.to_f64()), Some(q), "{n}/{d}");
            }
        }
    }
}
