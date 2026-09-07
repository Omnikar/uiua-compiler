#![allow(clippy::cast_precision_loss)]

use super::{AnalyzeContext, Error, ErrorKind, SingleAnalyzeResult, ValueInfo, types};
use types::ScalarInfo as S;

fn pervasive_dyadic(
    lhs_info: &ValueInfo,
    rhs_info: &ValueInfo,
    scalar_func: impl Fn(types::ScalarInfo, types::ScalarInfo) -> Result<types::ScalarInfo, Error>
    + Clone,
) -> SingleAnalyzeResult {
    todo!("Pervasion nonsense goes here")
}

#[derive(Clone, Copy)]
enum ScalarPair {
    Bool(Option<(bool, bool)>),
    Int(Option<(i64, i64)>),
    Float(Option<(f64, f64)>),
    Char(Option<(char, char)>),
}

fn promote_scalars(
    lhs_info: types::ScalarInfo,
    rhs_info: types::ScalarInfo,
    ctx: AnalyzeContext,
    func: impl Fn(ScalarPair) -> Result<types::ScalarInfo, Error>,
) -> Result<types::ScalarInfo, Error> {
    use ScalarPair as Sp;
    match (lhs_info, rhs_info) {
        (S::Bool(l), S::Bool(r)) => func(Sp::Bool(l.zip(r))),
        (S::Bool(l), S::Int(r)) => func(Sp::Int(l.map(Into::into).zip(r))),
        (S::Bool(l), S::Float(r)) => func(Sp::Float(l.map(Into::into).zip(r))),
        (S::Int(l), S::Bool(r)) => func(Sp::Int(l.zip(r.map(Into::into)))),
        (S::Int(l), S::Int(r)) => func(Sp::Int(l.zip(r))),
        (S::Int(l), S::Float(r)) => func(Sp::Float(l.map(|x| x as f64).zip(r))),
        (S::Float(l), S::Bool(r)) => func(Sp::Float(l.zip(r.map(Into::into)))),
        (S::Float(l), S::Int(r)) => func(Sp::Float(l.zip(r.map(|x| x as f64)))),
        (S::Float(l), S::Float(r)) => func(Sp::Float(l.zip(r))),
        (S::Char(l), S::Char(r)) => func(Sp::Char(l.zip(r))),
        (l, r) => ctx.error(ErrorKind::IncompatibleTypes(l.type_name(), r.type_name())),
    }
}

pub fn equals(
    lhs_info: &ValueInfo,
    rhs_info: &ValueInfo,
    ctx: AnalyzeContext,
) -> SingleAnalyzeResult {
    pervasive_dyadic(lhs_info, rhs_info, |lhs, rhs| {
        promote_scalars(lhs, rhs, ctx, |pair| {
            Ok(match pair {
                ScalarPair::Bool(pair) => S::Bool(pair.map(|(l, r)| l == r)),
                ScalarPair::Int(pair) => S::Bool(pair.map(|(l, r)| l == r)),
                #[allow(clippy::float_cmp)]
                ScalarPair::Float(pair) => S::Bool(pair.map(|(l, r)| l == r)),
                ScalarPair::Char(pair) => S::Bool(pair.map(|(l, r)| l == r)),
            })
        })
    })
}
