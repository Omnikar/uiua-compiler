mod pervasive_monadic;
mod pervasive_dyadic;

use super::{AnalyzeContext, Error, ErrorKind, TranslationContext, ValueInfo, types};
use crate::{hir, mir};

type SingleAnalyzeResult = Result<ValueInfo, Error>;

pub type MonadicImplFn = fn(&ValueInfo, AnalyzeContext) -> SingleAnalyzeResult;
pub fn monadic_prim(prim: hir::Prim) -> Option<MonadicImplFn> {
    use pervasive_monadic as pm;
    use uiua::ImplPrimitive as Ip;
    use uiua::Primitive as Pr;
    Some(match prim {
        hir::Prim::Prim(prim) => match prim {
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
        hir::Prim::Impl(impl_prim) => match impl_prim {
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

// pub type DyadicImplFn = fn(&ValueInfo, &ValueInfo, AnalyzeContext) -> SingleAnalyzeResult;
pub type DyadicImplFn =
    fn(&ValueInfo, &ValueInfo, AnalyzeContext, &mut TranslationContext) -> Result<(), Error>;
pub fn dyadic_prim(prim: hir::Prim) -> Option<DyadicImplFn> {
    use pervasive_dyadic as pd;
    use uiua::ImplPrimitive as Ip;
    use uiua::Primitive as Pr;
    Some(match prim {
        hir::Prim::Prim(prim) => match prim {
            Pr::Eq => pd::equals,
            _ => return None,
        },
        hir::Prim::Impl(impl_prim) => match impl_prim {
            _ => return None,
        },
    })
}
