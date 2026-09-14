mod pervasive_monadic;
mod pervasive_dyadic;

use super::{AnalyzeContext, Error, ErrorKind, FunctionTranslation, TirValue, ValueInfo, types};
use crate::{uir, tir};FunctionTranslator

use uiua::{ImplPrimitive as Ip, Primitive as Pr};

pub type MonadicImplFn = fn(&ValueInfo, AnalyzeContext) -> Result<ValueInfo, Error>;
pub fn monadic_prim(prim: uir::Prim) -> Option<MonadicImplFn> {
    use pervasive_monadic as pm;
    Some(match prim {
        uir::Prim::Prim(prim) => match prim {
            Pr::Not => pm::not,
            Pr::Sign => pm::sign,
            Pr::Neg => pm::negate,
            Pr::Reciprocal => pm::reciprocal,
            Pr::Abs => pm::absolute_value,
            Pr::Sqrt => pm::sqrt,
            Pr::Exp => pm::exponential,
            Pr::Sin => pm::sine,
            Pr::Cos => pm::cos,
            Pr::Tan => pm::tan,
            Pr::SinH => pm::sinh,
            Pr::CosH => pm::cosh,
            Pr::TanH => pm::tanh,
            Pr::Floor => pm::floor,
            Pr::Ceil => pm::ceiling,
            Pr::Round => pm::round,
            _ => return None,
        },
        uir::Prim::Impl(impl_prim) => match impl_prim {
            Ip::Ln => pm::ln,
            Ip::Log2 => pm::log2,
            Ip::Log10 => pm::log10,
            Ip::ASin => pm::asin,
            Ip::ACos => pm::acos,
            Ip::ATan => pm::atan,
            Ip::ASinH => pm::asinh,
            Ip::ACosH => pm::acosh,
            Ip::ATanH => pm::atanh,
            _ => return None,
        },
    })
}

pub type DyadicImplFn =
    fn(TirValue, TirValue, &FunctionTranslation, AnalyzeContext) -> Result<TirValue, Error>;
pub fn dyadic_prim(prim: uir::Prim) -> Option<DyadicImplFn> {
    use pervasive_dyadic as pd;
    Some(match prim {
        uir::Prim::Prim(prim) => match prim {
            Pr::Eq => pd::equals,
            Pr::Ne => pd::not_equals,
            Pr::Lt => pd::less_than,
            Pr::Le => pd::less_or_equal,
            Pr::Gt => pd::greater_than,
            Pr::Ge => pd::greater_or_equal,
            // TODO:
            // add
            // subtract
            // multiply
            Pr::Div => pd::divide,
            // moduloFunctionTranslator
            // power
            // minimum
            // maximum
            Pr::Atan => pd::atangent,
            _ => return None,
        },
        uir::Prim::Impl(impl_prim) => match impl_prim {
            Ip::Root => pd::root,
            // TODO: See https://github.com/uiua-lang/uiua/blob/8ff2203/src/impl_prim.rs#L107-L304
            _ => return None,
        },
    })
}
