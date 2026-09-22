mod polynomial;

use derive_more::From;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

pub use polynomial::Expr;

/// Typed IR, created via static analysis of UIR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tir {
    pub structs: Vec<Struct>,
    pub enums: Vec<Enum>,
    pub bindings: Vec<Binding>,
    pub main: Option<(Function, usize)>,
    pub spans: Vec<uiua::Span>,
    pub files: Rc<HashMap<PathBuf, String>>,
}

impl std::fmt::Display for Tir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ron = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new()).unwrap();
        let ron = crate::generic_ir::flatten_ron_number_lists(&ron);
        write!(f, "{ron}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub span: uiua::CodeSpan,
    pub func_id: uiua::FunctionId,
    pub hash: u64,
    pub func: Function,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Struct {
    pub name: String,
    pub info: types::StructInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Enum {
    pub name: String,
    pub info: types::EnumInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, From, Serialize, Deserialize)]
pub enum Prim {
    Prim(uiua::Primitive),
    Impl(uiua::ImplPrimitive),
}
impl From<crate::uir::Prim> for Prim {
    fn from(prim: crate::uir::Prim) -> Self {
        match prim {
            crate::uir::Prim::Prim(prim) => Self::Prim(prim),
            crate::uir::Prim::Impl(impl_prim) => Self::Impl(impl_prim),
        }
    }
}
impl From<&crate::uir::Prim> for Prim {
    fn from(prim: &crate::uir::Prim) -> Self {
        Self::from(*prim)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TirOp {
    CastNum {
        from: types::ScalarInfo,
        to: types::ScalarInfo,
    },
    // Check a particular axis length against a constant
    CheckAxis {
        depth: usize,
        ax_i: usize,
        length: usize,
    },
    // Check that a particular axis length in two inputs matches
    CheckAxes {
        lhs_depth: usize,
        lhs_ax_i: usize,
        rhs_depth: usize,
        rhs_ax_i: usize,
    },
}

#[derive(Debug, Clone, From, Serialize, Deserialize)]
pub enum Node {
    Input,
    Output,
    Constant(ValueInfo),
    FuncPrim(Prim),
    TirOp(TirOp),
    ModPrim(Prim, Vec<Function>),
    Call(usize),
    // ...
}
impl crate::generic_ir::FunctionNode for Node {
    fn is_input(&self) -> bool {
        matches!(self, Self::Input)
    }
    fn is_output(&self) -> bool {
        matches!(self, Self::Output)
    }
    fn input() -> Self {
        Self::Input
    }
    fn output() -> Self {
        Self::Output
    }
}

/// Values output by a node
pub type NodeMeta = Vec<ValueInfo>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionMeta {
    pub inputs: Vec<ValueInfo>,
    pub outputs: Vec<ValueInfo>,
}

pub type Function = crate::generic_ir::Function<FunctionMeta, Node, NodeMeta>;

/// Symbolic shape
pub type SymShape = Vec<Expr>;

pub fn fmt_shape(shape: &[Expr]) -> String {
    shape
        .iter()
        .map(|ax| ax.as_const().map_or_else(|| "?".into(), |x| x.to_string()))
        .join("×")
}

pub fn demote_known_shape(known_shape: &[usize]) -> Vec<Expr> {
    known_shape.iter().copied().map(Expr::from).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValueInfo {
    Scalar(types::ScalarInfo),
    Array(Box<types::ArrayInfo>),
    Map(Box<types::MapInfo>),
    Struct(types::StructInfo),
    Enum(types::EnumInfo),
    // TODO: File handles, etc?
}

impl ValueInfo {
    pub fn scalar_type(&self) -> Option<types::ScalarInfo> {
        match self {
            ValueInfo::Scalar(scalar) => Some(*scalar),
            ValueInfo::Array(array) => array.scalar_type(),
            ValueInfo::Map(map) => map.value_type.scalar_type(),
            _ => None,
        }
    }

    pub fn type_name(&self) -> Rc<str> {
        match self {
            ValueInfo::Scalar(scalar) => scalar.type_name().into(),
            ValueInfo::Array(array) => array.type_name(),
            ValueInfo::Map(map) => map.type_name(),
            ValueInfo::Struct(_) => todo!(),
            ValueInfo::Enum(_) => todo!(),
        }
    }

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
                if let Some(st) = scalar_supertype_ident!(lhs, rhs; S::Bool, S::Float, S::Char) {
                    return Some(Self::Scalar(st));
                } else if let (S::Int(l, linf), S::Int(r, rinf)) = (lhs, rhs) {
                    return Some(Self::Scalar(S::Int(
                        if l == r { *l } else { None },
                        *linf || *rinf,
                    )));
                }
                match (lhs, rhs) {
                    (S::Bool(b), S::Int(i, inf)) | (S::Int(i, inf), S::Bool(b)) => {
                        Some(Self::Scalar(S::Int(
                            b.map(i64::from).and_then(|b| i.filter(|i| b == *i)),
                            *inf,
                        )))
                    }
                    (S::Bool(b), other) | (other, S::Bool(b)) => {
                        Self::Scalar(S::Int(b.map(i64::from), false))
                            .supertype(&Self::Scalar(*other))
                    }
                    #[expect(clippy::float_cmp)]
                    (S::Int(i, inf), S::Float(f)) | (S::Float(f), S::Int(i, inf)) => {
                        Some(Self::Scalar(S::Float(
                            S::int_to_float(*i, *inf).and_then(|i| f.filter(|f| i == *f)),
                        )))
                    }
                    _ => None,
                }
            }

            (Self::Array(lhs), Self::Array(rhs)) => {
                lhs.supertype(rhs).map(Box::new).map(Self::Array)
            }
            _ => None,
        }
    }

    pub fn with_prepended_first_axis(&self, ax: Expr) -> Self {
        Self::Array(Box::new(match self {
            Self::Array(array_info) => match &**array_info {
                types::ArrayInfo::Known {
                    element_type,
                    value,
                } => types::ArrayInfo::Ranked {
                    element_type: element_type.clone(),
                    shape: [ax]
                        .into_iter()
                        .chain(value.shape.iter().copied().map(Expr::from))
                        .collect(),
                },
                types::ArrayInfo::Ranked {
                    element_type,
                    shape,
                } => types::ArrayInfo::Ranked {
                    element_type: element_type.clone(),
                    shape: [ax].into_iter().chain(shape.iter().cloned()).collect(),
                },
                types::ArrayInfo::Unranked {
                    element_type,
                    shape_prefix,
                    shape_suffix,
                } => types::ArrayInfo::Unranked {
                    element_type: element_type.clone(),
                    shape_prefix: [ax]
                        .into_iter()
                        .chain(shape_prefix.iter().cloned())
                        .collect(),
                    shape_suffix: [Expr::new_var()]
                        .into_iter()
                        .chain(shape_suffix.iter().cloned())
                        .collect(),
                },
            },
            _ => types::ArrayInfo::Ranked {
                element_type: self.clone(),
                shape: [ax.clone()].into(),
            },
        }))
    }
}

pub mod types {
    use itertools::Itertools;
    use serde::{Deserialize, Serialize};
    use std::rc::Rc;

    use super::{SymShape, ValueInfo};
    use crate::tir::polynomial::Expr;

    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub enum ScalarInfo {
        Bool(Option<bool>),
        /// Bool stores whether this represents integer-or-infinity
        Int(Option<i64>, bool),
        Float(Option<f64>),
        Char(Option<char>),
    }

    impl ScalarInfo {
        pub fn type_name(&self) -> &'static str {
            match self {
                ScalarInfo::Bool(_) => "boolean",
                ScalarInfo::Int(_, maybe_inf) => {
                    if *maybe_inf {
                        "integer-or-infinity"
                    } else {
                        "integer"
                    }
                }
                ScalarInfo::Float(_) => "float",
                ScalarInfo::Char(_) => "character",
            }
        }

        #[expect(clippy::cast_precision_loss)]
        pub fn int_to_float(val: Option<i64>, maybe_inf: bool) -> Option<f64> {
            val.map(|val| {
                if maybe_inf && val == i64::MAX {
                    f64::INFINITY
                } else if maybe_inf && val == i64::MIN + 1 {
                    f64::NEG_INFINITY
                } else {
                    val as f64
                }
            })
        }

        #[expect(clippy::cast_possible_truncation)]
        pub fn float_to_int(val: Option<f64>) -> (Option<i64>, bool) {
            let int_val = val.map(|val| {
                let int_val = val as i64;
                if val == f64::NEG_INFINITY {
                    int_val + 1
                } else {
                    int_val
                }
            });
            let maybe_inf = val.is_none_or(f64::is_infinite);
            (int_val, maybe_inf)
        }

        pub fn map_inf(
            val: Option<i64>,
            maybe_inf: bool,
            f: impl FnOnce(i64) -> i64,
        ) -> Option<i64> {
            val.map(|val| {
                if maybe_inf && (val == i64::MIN + 1 || val == i64::MAX) {
                    val
                } else {
                    f(val)
                }
            })
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ArrayValue {
        pub shape: Vec<usize>,
        pub data: Vec<ValueInfo>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ArrayInfo {
        /// Exact value known at compile time
        Known {
            element_type: ValueInfo,
            value: ArrayValue,
        },
        /// Rank known at compile time
        Ranked {
            element_type: ValueInfo,
            shape: SymShape,
        },
        /// Rank not known at compile time
        /// prefix, suffix
        Unranked {
            element_type: ValueInfo,
            shape_prefix: SymShape,
            shape_suffix: SymShape,
        },
    }

    impl ArrayInfo {
        pub fn element_type(&self) -> &ValueInfo {
            match self {
                Self::Known { element_type, .. }
                | Self::Ranked { element_type, .. }
                | Self::Unranked { element_type, .. } => element_type,
            }
        }

        pub fn scalar_type(&self) -> Option<ScalarInfo> {
            self.element_type().scalar_type()
        }

        pub fn sym_shape(&self) -> Option<std::borrow::Cow<'_, [Expr]>> {
            match self {
                Self::Known { value, .. } => Some(super::demote_known_shape(&value.shape).into()),
                Self::Ranked { shape, .. } => Some(shape.into()),
                _ => None,
            }
        }

        pub fn type_name(&self) -> Rc<str> {
            let shape_s = match self {
                Self::Known { value, .. } => {
                    let s = value.shape.iter().map(ToString::to_string).join("×");
                    format!("shape {s} ")
                }
                Self::Ranked { shape, .. } => {
                    let s = super::fmt_shape(shape);
                    format!("shape {s} ")
                }
                Self::Unranked { .. } => String::new(),
            };
            let element_type_s = self.element_type().type_name();
            format!("{shape_s}array of {element_type_s}").into()
        }

        #[expect(
            clippy::too_many_lines,
            reason = "This function is one big `match` expression that it doesn't seem can be split up very ergonomically."
        )]
        pub fn supertype(&self, rhs: &Self) -> Option<Self> {
            match (self, rhs) {
                (
                    Self::Known {
                        element_type: lhs_element_type,
                        value: lhs_value,
                    },
                    Self::Known {
                        element_type: rhs_element_type,
                        value: rhs_value,
                    },
                ) => {
                    let element_type = lhs_element_type.supertype(rhs_element_type)?;
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
                                element_type,
                                value: ArrayValue {
                                    shape: fixed_shape,
                                    data,
                                },
                            })
                        } else {
                            Some(Self::Ranked {
                                element_type,
                                shape,
                            })
                        }
                    } else {
                        todo!("Unranked supertypes are not yet implemented")
                    }
                }
                (
                    Self::Ranked {
                        element_type: lhs_element_type,
                        shape: lhs_shape,
                    },
                    Self::Ranked {
                        element_type: rhs_element_type,
                        shape: rhs_shape,
                    },
                ) => {
                    let element_type = lhs_element_type.supertype(rhs_element_type)?;
                    if lhs_shape.len() == rhs_shape.len() {
                        let shape = lhs_shape
                            .iter()
                            .zip(rhs_shape)
                            .map(|(a, b)| if a == b { a.clone() } else { Expr::new_var() })
                            .collect_vec();
                        Some(Self::Ranked {
                            element_type,
                            shape,
                        })
                    } else {
                        todo!("Unranked supertypes are not yet implemented")
                    }
                }
                (
                    Self::Unranked {
                        element_type: lhs_element_type,
                        shape_prefix: _lhs_shape_prefix,
                        shape_suffix: _lhs_shape_suffix,
                    },
                    Self::Unranked {
                        element_type: rhs_element_type,
                        shape_prefix: _rhs_shape_prefix,
                        shape_suffix: _rhs_shape_suffix,
                    },
                ) => {
                    let _element_type = lhs_element_type.supertype(rhs_element_type)?;
                    todo!("Unranked supertypes are not yet implemented")
                }
                (
                    Self::Known {
                        element_type: known_element_type,
                        value,
                    },
                    other,
                )
                | (
                    other,
                    Self::Known {
                        element_type: known_element_type,
                        value,
                    },
                ) => Self::Ranked {
                    element_type: known_element_type.clone(),
                    shape: value.shape.iter().copied().map(From::from).collect(),
                }
                .supertype(other),
                _ => todo!(),
            }
        }

        pub fn split_first_axis(&self) -> Option<(Expr, ValueInfo)> {
            Some(match self {
                Self::Known {
                    element_type,
                    value,
                } => (
                    value.shape[0].into(),
                    match value.shape.len() {
                        1 => element_type.clone(),
                        2.. => ValueInfo::Array(Box::new(Self::Ranked {
                            element_type: element_type.clone(),
                            shape: value.shape[1..].iter().copied().map(Expr::from).collect(),
                        })),
                        _ => return None,
                    },
                ),
                Self::Ranked {
                    element_type,
                    shape,
                } => (
                    shape[0].clone(),
                    match shape.len() {
                        1 => element_type.clone(),
                        2.. => ValueInfo::Array(Box::new(Self::Ranked {
                            element_type: element_type.clone(),
                            shape: shape[1..].to_vec(),
                        })),
                        _ => return None,
                    },
                ),
                _ => return None,
            })
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MapInfo {
        pub key_type: ValueInfo,
        pub value_type: ValueInfo,
    }

    impl MapInfo {
        pub fn type_name(&self) -> Rc<str> {
            format!(
                "map from {} to {}",
                self.key_type.type_name(),
                self.value_type.type_name(),
            )
            .into()
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct StructInfo {
        pub fields: Rc<[(String, ValueInfo)]>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct EnumInfo {
        pub variants: Rc<[(String, StructInfo)]>,
    }
}
