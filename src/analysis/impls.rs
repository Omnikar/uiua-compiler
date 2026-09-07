mod pervasive_monadic;

use super::{AnalyzeContext, Error, ErrorKind, ValueInfo, types};

type SingleAnalyzeResult = Result<ValueInfo, Error>;

pub type MonadicImplFn = fn(&ValueInfo, AnalyzeContext) -> SingleAnalyzeResult;
pub fn monadic_impl(prim: uiua::Primitive) -> Option<MonadicImplFn> {
    use pervasive_monadic as pm;
    use uiua::Primitive as Pr;
    Some(match prim {
        Pr::Not => pm::not,
        Pr::Sign => pm::sign,
        Pr::Neg => pm::negate,
        Pr::Reciprocal => pm::reciprocal,
        Pr::Abs => pm::absolute_value,
        Pr::Sqrt => pm::sqrt,
        _ => return None,
    })
}
