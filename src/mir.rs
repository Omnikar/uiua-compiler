pub mod polynomial;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use polynomial::Expr;

#[derive(Debug, Clone)]
pub struct Mir {
    pub structs: Vec<Struct>,
    pub enums: Vec<Enum>,
    pub bindings: Vec<Binding>,
    pub spans: Vec<uiua::Span>,
    pub files: HashMap<PathBuf, String>,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub span: uiua::CodeSpan,
    pub func_id: uiua::FunctionId,
    pub hash: u64,
    pub func: Function,
}

#[derive(Debug, Clone)]
pub struct Struct {
    pub name: String,
    pub info: types::StructInfo,
}

#[derive(Debug, Clone)]
pub struct Enum {
    pub name: String,
    pub info: types::EnumInfo,
}

#[derive(Debug, Clone)]
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

/// Values output by a node
pub type NodeMeta = Vec<ValueInfo>;

#[derive(Debug, Clone)]
pub struct FunctionMeta {
    inputs: Vec<ValueInfo>,
    outputs: Vec<ValueInfo>,
}

pub type Function = crate::generic_ir::Function<FunctionMeta, Node, NodeMeta>;

/// Symbolic shape
pub type SymShape = Vec<Expr>;

#[derive(Debug, Clone)]
pub enum ValueInfo {
    Bool(Option<bool>),
    Int(Option<i64>),
    Float(Option<f64>),
    Char(Option<char>),
    Array(Box<types::ArrayInfo>),
    Map(Box<types::MapInfo>),
    Struct(types::StructInfo),
    Enum(types::EnumInfo),
    // TODO: File handles, etc?
}

pub mod types {
    use super::{SymShape, ValueInfo};
    use std::rc::Rc;

    #[derive(Debug, Clone)]
    pub struct ArrayValue {
        shape: Vec<usize>,
        data: Vec<ValueInfo>,
    }

    #[derive(Debug, Clone)]
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

    #[derive(Debug, Clone)]
    pub struct MapInfo {
        key_type: ValueInfo,
        value_type: ValueInfo,
    }

    #[derive(Debug, Clone)]
    pub struct StructInfo {
        fields: Rc<[(String, ValueInfo)]>,
    }

    #[derive(Debug, Clone)]
    pub struct EnumInfo {
        variants: Rc<[(String, StructInfo)]>,
    }
}
