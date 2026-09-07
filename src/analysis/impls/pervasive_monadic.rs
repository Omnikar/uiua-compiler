#![allow(clippy::cast_precision_loss)]

use super::{AnalyzeContext, Error, ErrorKind, SingleAnalyzeResult, ValueInfo, types};

use types::ScalarInfo as S;

fn pervasive_monadic(
    input_info: &ValueInfo,
    scalar_func: impl Fn(&types::ScalarInfo) -> Result<types::ScalarInfo, Error> + Clone,
) -> SingleAnalyzeResult {
    Ok(match input_info {
        ValueInfo::Scalar(scalar_info) => scalar_func(scalar_info).map(ValueInfo::Scalar)?,
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

pub fn not(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(b) => S::Bool(b.map(|b| !b)),
            S::Int(i) => S::Int(i.map(|i| 1 - i)),
            S::Float(f) => S::Float(f.map(|f| 1.0 - f)),
            S::Char(_) => ctx.error(ErrorKind::NotChar)?,
        })
    })
}

pub fn sign(input_info: &ValueInfo, _ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) => *scalar,
            S::Int(i) => S::Int(i.map(i64::signum)),
            S::Float(f) => S::Float(f.map(|f| if f == 0.0 { 0.0 } else { f.signum() })),
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

pub fn reciprocal(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(b) => S::Float(b.map(|b| f64::from(b).recip())),
            S::Int(i) => S::Float(i.map(|i| (i as f64).recip())),
            S::Float(f) => S::Float(f.map(f64::recip)),
            S::Char(_) => ctx.error(ErrorKind::RecipChar)?,
        })
    })
}

pub fn absolute_value(input_info: &ValueInfo, _ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) => *scalar,
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

pub fn sqrt(input_info: &ValueInfo, ctx: AnalyzeContext) -> SingleAnalyzeResult {
    pervasive_monadic(input_info, |scalar| {
        Ok(match scalar {
            S::Bool(_) => *scalar,
            S::Int(i) => S::Float(i.map(|i| (i as f64).sqrt())),
            S::Float(f) => S::Float(f.map(f64::sqrt)),
            S::Char(_) => ctx.error(ErrorKind::SqrtChar)?,
        })
    })
}
