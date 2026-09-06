use itertools::Itertools;
use std::collections::HashMap;
use std::ops::{Add, Mul, Sub};
use std::rc::Rc;

thread_local! {
    static NVARS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A multivariate polynomial of arbitrarily many variables, used to track relations between array axes of unknown length
///
/// The polynomial is represented as a hashmap from exponent values to coefficients. For example, an entry of `[1, 2] -> 3` represents the term `3x₀x₁²` in the polynomial.
/// No stored coefficients should be zero, and no exponent lists should have trailing zeros.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    terms: HashMap<Rc<[u32]>, isize>,
}

impl Expr {
    /// Create a new variable that has never been created before
    pub fn new_var() -> Self {
        let nvars = NVARS.get();
        let mut exponents = vec![0; nvars];
        exponents.push(1);
        NVARS.set(nvars + 1);
        Self {
            terms: [(exponents.into(), 1)].into(),
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
impl From<usize> for Expr {
    fn from(value: usize) -> Self {
        Self {
            terms: [([].into(), value.try_into().unwrap())].into(),
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
                            .iter()
                            .copied()
                            .zip_longest(rexps.iter().copied())
                            .map(itertools::EitherOrBoth::or_default)
                            .map(|(l, r)| l + r)
                            .collect::<Rc<[_]>>(),
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
impl Mul<isize> for Expr {
    type Output = Self;
    fn mul(mut self, rhs: isize) -> Self::Output {
        self.terms.iter_mut().for_each(|(_, coef)| *coef *= rhs);
        self
    }
}
impl Mul<Expr> for isize {
    type Output = Expr;
    fn mul(self, rhs: Expr) -> Self::Output {
        rhs * self
    }
}
impl std::iter::Product for Expr {
    fn product<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(1isize.into(), |x, y| x * y)
    }
}
impl Expr {
    pub fn pow(self, n: u32) -> Self {
        std::iter::repeat_n(self, n as usize).product()
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self
            .terms
            .iter()
            .map(|(exps, coef)| {
                let vars = exps
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &p)| {
                        if p == 0 {
                            None
                        } else if p == 1 {
                            Some(format!("x{}", encode_num(i, &SUBSCRIPT_CHARS)))
                        } else {
                            Some(format!(
                                "x{}{}",
                                encode_num(i, &SUBSCRIPT_CHARS),
                                encode_num(p as usize, &SUPERSCRIPT_CHARS)
                            ))
                        }
                    })
                    .join("");
                if *coef > 1 {
                    format!("{coef}{vars}")
                } else {
                    vars
                }
            })
            .join(" + ");
        write!(f, "{s}")
    }
}
impl serde::Serialize for Expr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

const SUBSCRIPT_CHARS: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];
const SUPERSCRIPT_CHARS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
fn encode_num(num: usize, chars: &[char; 10]) -> String {
    num.to_string()
        .chars()
        .map(|c| chars[c.to_digit(10).unwrap() as usize])
        .collect()
}
