mod pervasive_monadic;

use super::{AnalyzeContext, Error, ErrorKind, ValueInfo, types};

type SingleAnalyzeResult = Result<ValueInfo, Error>;

pub type MonadicImplFn = fn(&ValueInfo, AnalyzeContext) -> SingleAnalyzeResult;
pub fn monadic_prim(prim: uiua::Primitive) -> Option<MonadicImplFn> {
    use pervasive_monadic as pm;
    use uiua::Primitive as Pr;
    Some(match prim {
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
    })
}
pub fn monadic_impl_prim(impl_prim: uiua::ImplPrimitive) -> Option<MonadicImplFn> {
    use pervasive_monadic as pm;
    use uiua::ImplPrimitive as Ip;
    Some(match impl_prim {
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
    })
}
