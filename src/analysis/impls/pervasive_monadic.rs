#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use super::{AnalyzeContext, Error, ErrorKind, SingleAnalyzeResult, ValueInfo, types};
use types::ScalarInfo as S;

fn pervasive_monadic(
    input_info: &ValueInfo,
    scalar_func: impl Fn(types::ScalarInfo) -> Result<types::ScalarInfo, Error> + Clone,
) -> SingleAnalyzeResult {
    Ok(match input_info {
        ValueInfo::Scalar(scalar_info) => scalar_func(*scalar_info).map(ValueInfo::Scalar)?,
        ValueInfo::Array(array_info) => match &**array_info {
            types::ArrayInfo::Known { scalar_type, value } => {
                // TODO: Size limit for pre-evaluation?
                let scalar_type = pervasive_monadic(scalar_type, scalar_func.clone())?;
                let value = types::ArrayValue {
                    shape: value.shape.clone(),
                    data: value
                        .data
                        .iter()
                        .map(|x| pervasive_monadic(x, scalar_func.clone()))
                        .collect::<Result<Vec<_>, _>>()?,
                };
                ValueInfo::Array(Box::new(types::ArrayInfo::Known { scalar_type, value }))
            }
            types::ArrayInfo::Ranked { scalar_type, shape } => {
                let scalar_type = pervasive_monadic(scalar_type, scalar_func)?;
                ValueInfo::Array(Box::new(types::ArrayInfo::Ranked {
                    scalar_type,
                    shape: shape.clone(),
                }))
            }
            types::ArrayInfo::Unranked {
                scalar_type,
                shape_prefix,
                shape_suffix,
            } => {
                let scalar_type = pervasive_monadic(scalar_type, scalar_func)?;
                ValueInfo::Array(Box::new(types::ArrayInfo::Unranked {
                    scalar_type,
                    shape_prefix: shape_prefix.clone(),
                    shape_suffix: shape_suffix.clone(),
                }))
            }
        },
        ValueInfo::Map(map_info) => ValueInfo::Map(Box::new(types::MapInfo {
            key_type: map_info.key_type.clone(),
            value_type: pervasive_monadic(&map_info.value_type, scalar_func)?,
        })),
        _ => todo!(),
    })
}

fn float_func(
    input_info: &ValueInfo,
    ctx: AnalyzeContext,
    func: fn(f64) -> f64,
    error: ErrorKind,
) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(b) => S::Float(b.map(|b| func(f64::from(b)))),
            S::Int(i) => S::Float(i.map(|i| func(i as f64))),
            S::Float(f) => S::Float(f.map(func)),
            S::Char(_) => ctx.error(error)?,
        })
    })
}

pub fn not(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(b) => S::Bool(b.map(|b| !b)),
            S::Int(i) => S::Int(i.map(|i| 1 - i)),
            S::Float(f) => S::Float(f.map(|f| 1.0 - f)),
            S::Char(_) => ctx.error(ErrorKind::ExpectedNumber("not"))?,
        })
    })
}

pub fn sign(input_info: &ValueInfo, _ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) => scalar,
            S::Int(i) => S::Int(i.map(i64::signum)),
            S::Float(f) => S::Int(f.map(|f| if f == 0.0 { 0 } else { f.signum() as i64 })),
            S::Char(c) => {
                S::Int(c.map(|c| i64::from(c.is_uppercase()) - i64::from(c.is_lowercase())))
            }
        })
    })
}

pub fn negate(input_info: &ValueInfo, _ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(b) => S::Int(b.map(|b| -i64::from(b))),
            S::Int(i) => S::Int(i.map(|i| -i)),
            S::Float(f) => S::Float(f.map(|f| -f)),
            S::Char(c) => S::Char(c.map(|c| {
                if c.is_uppercase()
                    && let mut lower = c.to_lowercase()
                    && lower.len() == 1
                {
                    lower.next().unwrap()
                } else if c.is_lowercase()
                    && let mut upper = c.to_uppercase()
                    && upper.len() == 1
                {
                    upper.next().unwrap()
                } else {
                    c
                }
            })),
        })
    })
}

pub fn absolute_value(input_info: &ValueInfo, _ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) => scalar,
            S::Int(i) => S::Int(i.map(i64::abs)),
            S::Float(f) => S::Float(f.map(f64::abs)),
            S::Char(c) => S::Char(c.map(|c| {
                let mut upper = c.to_uppercase();
                if upper.len() == 1 {
                    upper.next().unwrap()
                } else {
                    c
                }
            })),
        })
    })
}

pub fn floor(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) | S::Int(_) => scalar,
            S::Float(f) => S::Int(f.map(|f| f.floor() as i64)),
            S::Char(_) => ctx.error(ErrorKind::ExpectedNumber("floor"))?,
        })
    })
}

pub fn ceiling(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) | S::Int(_) => scalar,
            S::Float(f) => S::Int(f.map(|f| f.ceil() as i64)),
            S::Char(_) => ctx.error(ErrorKind::ExpectedNumber("ceiling"))?,
        })
    })
}

pub fn round(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) | S::Int(_) => scalar,
            S::Float(f) => S::Int(f.map(|f| f.round() as i64)),
            S::Char(_) => ctx.error(ErrorKind::ExpectedNumber("round"))?,
        })
    })
}

/// Define analyses for functions that always expect numerical inputs
/// and always produce floating-point outputs
macro_rules! float_funcs {
    ($($name:ident, $name_str:literal, $func:expr;)*) => {
        $(
            pub fn $name(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
                float_func(
                    input_info,
                    ctx,
                    $func,
                    ErrorKind::ExpectedNumber($name_str),
                )
            }
        )*
    };
}

float_funcs! {
    reciprocal, "reciprocal", f64::recip;
    sqrt, "square root", f64::sqrt;
    exponential, "exponential", f64::exp;
    sine, "sine", f64::sin;
    cos, "cosine", f64::cos;
    tan, "tangent", f64::tan;
    sinh, "hyperbolic sine", f64::sinh;
    cosh, "hyperbolic cosine", f64::cosh;
    tanh, "hyperbolic tangent", f64::tanh;

    ln, "natural logarithm", f64::ln;
    log2, "base-2 logarithm", f64::log2;
    log10, "base-10 logarithm", f64::log10;
    asin, "inverse sine", f64::asin;
    acos, "inverse cosine", f64::acos;
    atan, "inverse tangent", f64::atan;
    asinh, "inverse hyperbolic sine", f64::asinh;
    acosh, "inverse hyperbolic cosine", f64::acosh;
    atanh, "inverse hyperbolic tangent", f64::atanh;
}
