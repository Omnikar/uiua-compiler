pub mod polynomial;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use polynomial::Expr;

#[derive(Debug, Clone, Serialize)]
pub struct Mir {
    pub structs: Vec<Struct>,
    pub enums: Vec<Enum>,
    pub bindings: Vec<Binding>,
    pub main: Option<(Function, usize)>,
    pub spans: Vec<uiua::Span>,
    pub files: Rc<HashMap<PathBuf, String>>,
}

impl std::fmt::Display for Mir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ron = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new()).unwrap();
        let ron = crate::generic_ir::flatten_ron_number_lists(&ron);
        write!(f, "{ron}")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Binding {
    pub span: uiua::CodeSpan,
    pub func_id: uiua::FunctionId,
    pub hash: u64,
    pub func: Function,
}

#[derive(Debug, Clone, Serialize)]
pub struct Struct {
    pub name: String,
    pub info: types::StructInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct Enum {
    pub name: String,
    pub info: types::EnumInfo,
}

#[derive(Debug, Clone, Serialize)]
pub enum Node {
    Input,
    Output,
    Constant(ValueInfo),
    FuncPrim(uiua::Primitive),
    FuncImplPrim(uiua::ImplPrimitive),
    ModPrim(uiua::Primitive, Vec<Function>),
    ModImplPrim(uiua::ImplPrimitive, Vec<Function>),
    // Call(…),
    // ...
}

/// Values output by a node
pub type NodeMeta = Vec<ValueInfo>;

#[derive(Debug, Clone, Serialize)]
pub struct FunctionMeta {
    pub inputs: Vec<ValueInfo>,
    pub outputs: Vec<ValueInfo>,
}

pub type Function = crate::generic_ir::Function<FunctionMeta, Node, NodeMeta>;

/// Symbolic shape
pub type SymShape = Vec<Expr>;

#[derive(Debug, Clone, Serialize)]
pub enum ValueInfo {
    Scalar(types::ScalarInfo),
    Array(Box<types::ArrayInfo>),
    Map(Box<types::MapInfo>),
    Struct(types::StructInfo),
    Enum(types::EnumInfo),
    // TODO: File handles, etc?
}

impl ValueInfo {
    pub fn supertype(&self, rhs: &Self) -> Option<Self> {
        macro_rules! scalar_supertype_ident {
            ($lhs:expr, $rhs:expr; $($variant:path),+) => {
                match ($lhs, $rhs) {
                    $(
                        ($variant(l), $variant(r)) => {
                            if l == r {
                                Some($variant(*l))
                            } else {
                                Some($variant(None))
                            }
                        }
                    )+
                    _ => None
                }
            };
        }
        match (self, rhs) {
            (Self::Scalar(lhs), Self::Scalar(rhs)) => {
                use types::ScalarInfo as S;
                if let Some(st) =
                    scalar_supertype_ident!(lhs, rhs; S::Bool, S::Int, S::Float, S::Char)
                {
                    return Some(Self::Scalar(st));
                }
                match (lhs, rhs) {
                    (S::Bool(b), S::Int(i)) | (S::Int(i), S::Bool(b)) => Some(Self::Scalar(
                        S::Int(b.map(i64::from).and_then(|b| i.filter(|i| b == *i))),
                    )),
                    (S::Bool(b), other) | (other, S::Bool(b)) => {
                        Self::Scalar(S::Int(b.map(i64::from))).supertype(&Self::Scalar(*other))
                    }
                    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
                    (S::Int(i), S::Float(f)) | (S::Float(f), S::Int(i)) => Some(Self::Scalar(
                        S::Float(i.map(|i| i as f64).and_then(|i| f.filter(|f| i == *f))),
                    )),
                    _ => None,
                }
            }

            (Self::Array(lhs), Self::Array(rhs)) => {
                lhs.supertype(rhs).map(Box::new).map(Self::Array)
            }
            _ => None,
        }
    }
}

pub mod types {
    use itertools::Itertools;
    use serde::{Deserialize, Serialize};
    use std::rc::Rc;

    use super::{SymShape, ValueInfo};
    use crate::mir::polynomial::Expr;

    #[derive(Debug, Clone, Copy, Serialize)]
    pub enum ScalarInfo {
        Bool(Option<bool>),
        Int(Option<i64>),
        Float(Option<f64>),
        Char(Option<char>),
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct ArrayValue {
        pub shape: Vec<usize>,
        pub data: Vec<ValueInfo>,
    }

    #[derive(Debug, Clone, Serialize)]
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

    impl ArrayInfo {
        #[allow(
            clippy::too_many_lines,
            reason = "This function is one big `match` expression that it doesn't seem can be split up very ergonomically."
        )]
        pub fn supertype(&self, rhs: &Self) -> Option<Self> {
            match (self, rhs) {
                (
                    Self::Known {
                        scalar_type: lhs_scalar_type,
                        value: lhs_value,
                    },
                    Self::Known {
                        scalar_type: rhs_scalar_type,
                        value: rhs_value,
                    },
                ) => {
                    let scalar_type = lhs_scalar_type.supertype(rhs_scalar_type)?;
                    if lhs_value.shape.len() == rhs_value.shape.len() {
                        let shape = lhs_value
                            .shape
                            .iter()
                            .zip(&rhs_value.shape)
                            .map(|(a, b)| {
                                if a == b {
                                    Expr::from(*a)
                                } else {
                                    Expr::new_var()
                                }
                            })
                            .collect_vec();
                        if let Some(fixed_shape) = shape
                            .iter()
                            .map(|ax| ax.as_const().and_then(|ax| usize::try_from(ax).ok()))
                            .collect::<Option<Vec<_>>>()
                            && let Some(data) = lhs_value
                                .data
                                .iter()
                                .zip(&rhs_value.data)
                                .map(|(a, b)| a.supertype(b))
                                .collect::<Option<Vec<_>>>()
                        {
                            Some(Self::Known {
                                scalar_type,
                                value: ArrayValue {
                                    shape: fixed_shape,
                                    data,
                                },
                            })
                        } else {
                            Some(Self::Ranked { scalar_type, shape })
                        }
                    } else {
                        todo!("Unranked supertypes are not yet implemented")
                    }
                }
                (
                    Self::Ranked {
                        scalar_type: lhs_scalar_type,
                        shape: lhs_shape,
                    },
                    Self::Ranked {
                        scalar_type: rhs_scalar_type,
                        shape: rhs_shape,
                    },
                ) => {
                    let scalar_type = lhs_scalar_type.supertype(rhs_scalar_type)?;
                    if lhs_shape.len() == rhs_shape.len() {
                        let shape = lhs_shape
                            .iter()
                            .zip(rhs_shape)
                            .map(|(a, b)| if a == b { a.clone() } else { Expr::new_var() })
                            .collect_vec();
                        Some(Self::Ranked { scalar_type, shape })
                    } else {
                        todo!("Unranked supertypes are not yet implemented")
                    }
                }
                (
                    Self::Unranked {
                        scalar_type: lhs_scalar_type,
                        shape_prefix: _lhs_shape_prefix,
                        shape_suffix: _lhs_shape_suffix,
                    },
                    Self::Unranked {
                        scalar_type: rhs_scalar_type,
                        shape_prefix: _rhs_shape_prefix,
                        shape_suffix: _rhs_shape_suffix,
                    },
                ) => {
                    let _scalar_type = lhs_scalar_type.supertype(rhs_scalar_type)?;
                    todo!("Unranked supertypes are not yet implemented")
                }
                (
                    Self::Known {
                        scalar_type: known_scalar_type,
                        value,
                    },
                    other,
                )
                | (
                    other,
                    Self::Known {
                        scalar_type: known_scalar_type,
                        value,
                    },
                ) => Self::Ranked {
                    scalar_type: known_scalar_type.clone(),
                    shape: value.shape.iter().copied().map(From::from).collect(),
                }
                .supertype(other),
                _ => todo!(),
            }
        }
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct MapInfo {
        pub key_type: ValueInfo,
        pub value_type: ValueInfo,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct StructInfo {
        pub fields: Rc<[(String, ValueInfo)]>,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct EnumInfo {
        pub variants: Rc<[(String, StructInfo)]>,
    }
}
