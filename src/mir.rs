pub mod math;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use math::Expr;

pub struct Idk;

pub struct Mir {
    pub structs: Vec<Struct>,
    pub enums: Vec<Enum>,
    pub bindings: Vec<Binding>,
    pub spans: Vec<uiua::Span>,
    pub files: HashMap<PathBuf, String>,
}

pub struct Binding {
    pub span: uiua::CodeSpan,
    pub func_id: uiua::FunctionId,
    pub hash: u64,
    pub func: Function,
}

pub struct Struct {
    pub name: String,
    pub info: types::StructInfo,
}

pub struct Enum {
    pub name: String,
    pub info: types::EnumInfo,
}

pub enum Node {
    Input,
    Output,
    Constant(ValueInfo),
    FuncPrim(uiua::Primitive),
    FuncImplPrim(uiua::ImplPrimitive),
    ModPrim(uiua::Primitive, Vec<Function>),
    ModImplPrim(uiua::ImplPrimitive, Vec<Function>),
    // ...
}

pub type Function = crate::generic_ir::Function<Idk, Node, Idk>;

/// Symbolic shape
pub type SymShape = Vec<math::Expr>;

pub enum ValueInfo {
    Bool(Option<bool>),
    Int(Option<i64>),
    Float(Option<f64>),
    Char(Option<char>),
    Array(Box<types::ArrayInfo>),
    Map(Box<types::MapInfo>),
    Struct(types::StructInfo),
    Enum(types::EnumInfo),
}

mod types {
    use super::{SymShape, ValueInfo};
    use std::rc::Rc;

    pub struct ArrayValue {
        shape: Vec<usize>,
        data: Vec<ValueInfo>,
    }

    pub enum ArrayInfo {
        /// Exact value known at compile time
        Known {
            scalar_type: ValueInfo,
            value: ArrayValue,
        },
        /// Rank known at compile time
        Ranked {
            scalar_type: ValueInfo,
            shape: SymShape,
        },
        /// Rank not known at compile time
        /// prefix, suffix
        Unranked {
            scalar_type: ValueInfo,
            shape_prefix: SymShape,
            shape_suffix: SymShape,
        },
    }

    pub struct MapInfo {
        key_type: ValueInfo,
        value_type: ValueInfo,
    }

    pub struct StructInfo {
        fields: Rc<[(String, ValueInfo)]>,
    }

    pub struct EnumInfo {
        variants: Rc<[(String, StructInfo)]>,
    }
}

fn ababa() {
    let mut nvars = 0;
    let shape_n_jagged_array_of_m_by_2_int_arrays =
        ValueInfo::Array(Box::new(types::ArrayInfo::Ranked {
            scalar_type: ValueInfo::Array(Box::new(types::ArrayInfo::Ranked {
                scalar_type: ValueInfo::Int(None),
                shape: [Expr::new_var(&mut nvars), 2.into()].into(),
            })),
            shape: [Expr::new_var(&mut nvars)].into(),
        }));
}
