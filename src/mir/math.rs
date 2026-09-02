use itertools::Itertools;
use std::collections::HashMap;
use std::ops::{Add, Mul, Sub};

/// A multivariate polynomial of arbitrarily many variables, used to track relations between array axes of unknown length
///
/// The polynomial is represented as a hashmap from exponent values to coefficients. For example, an entry of `[1, 2] -> 3` represents the term `3x₀x₁²` in the polynomial.
/// No stored coefficients should be zero, and no exponent lists should have trailing zeros.
#[derive(Clone, Debug)]
pub struct Expr {
    terms: HashMap<Vec<u32>, isize>,
}

impl Expr {
    /// Given a variable counter, make a new variable with the next unoccupied index and increment the counter
    pub fn new_var(nvars: &mut usize) -> Self {
        let mut exponents = vec![0; *nvars];
        exponents.push(1);
        *nvars += 1;
        Self {
            terms: [(exponents, 1)].into(),
        }
    }

    /// If this expression has only a constant term, return it
    pub fn as_const(&self) -> Option<isize> {
        debug_assert!(self.terms.values().all(|coef| *coef != 0));
        match self.terms.get(&[] as &[u32]) {
            None if self.terms.is_empty() => Some(0),
            Some(coef) if self.terms.len() == 1 => Some(*coef),
            _ => None,
        }
    }
}

impl From<isize> for Expr {
    fn from(value: isize) -> Self {
        Self {
            terms: [([].into(), value)].into(),
        }
    }
}

impl Add for Expr {
    type Output = Self;
    fn add(mut self, mut rhs: Self) -> Self::Output {
        self.terms.retain(|exps, coef| {
            *coef += rhs.terms.remove(exps).unwrap_or(0);
            *coef != 0
        });
        self.terms.extend(rhs.terms);
        self
    }
}
impl Sub for Expr {
    type Output = Self;
    fn sub(mut self, mut rhs: Self) -> Self::Output {
        self.terms.retain(|exps, coef| {
            *coef -= rhs.terms.remove(exps).unwrap_or(0);
            *coef != 0
        });
        self.terms.extend(rhs.terms);
        self
    }
}
impl Mul for Expr {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            terms: self
                .terms
                .into_iter()
                .cartesian_product(rhs.terms.iter())
                .map(|((lexps, lcoef), (rexps, rcoef))| {
                    (
                        lexps
                            .into_iter()
                            .zip_longest(rexps.iter().copied())
                            .map(itertools::EitherOrBoth::or_default)
                            .map(|(l, r)| l + r)
                            .collect::<Vec<_>>(),
                        lcoef * *rcoef,
                    )
                })
                .fold(HashMap::new(), |mut map, (exps, coef)| {
                    *map.entry(exps).or_default() += coef;
                    map
                }),
        }
    }
}
