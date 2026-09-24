use itertools::Itertools;
use std::collections::HashMap;

use crate::analysis::{
    AnalyzeContext, Error, ErrorKind, FunctionTranslator, TirValue, monomorphize_and_analyze,
};
use crate::generic_ir::NodeIndex;
use crate::tir::{self, Expr, ValueInfo, types};

pub fn translate_function_call(
    uiua_func: &uiua::Function,
    inputs: &[TirValue],
    ctx: AnalyzeContext,
    uir_node_idx: NodeIndex,
    tr: &FunctionTranslator,
) -> Result<(), Error> {
    // TODO: Inlining?
    // TODO: Recursion?

    let uir_binding = tr
        .uir
        .bindings
        .iter()
        .find(|binding| binding.func_id == uiua_func.id)
        .unwrap();
    let uir_func = &uir_binding.func;

    let mut substs = Vec::new();

    let mut tir = tr.tir.borrow();
    let (i, binding) = if let Some((i, binding)) =
        tir.bindings.iter().enumerate().find(|(_, binding)| {
            let mut new_substs = Vec::new();
            let found = uiua_func.hash() == binding.hash
                && inputs.len() == binding.func.meta.inputs.len()
                && tr.infos_dyn(inputs).zip(&binding.func.meta.inputs).all(
                    |(input_info, func_input_info)| {
                        input_info
                            .match_monomorphization(func_input_info)
                            .map(|substs| new_substs.extend(substs))
                            .is_some()
                    },
                );
            if found {
                substs.extend(new_substs);
            }
            found
        }) {
        (i, binding)
    } else {
        // Release the `RefCell` so that we can pass it to the recursive
        // `monomorphize_and_analyze` call safely
        drop(tir);
        let input_infos = tr.infos_dyn(inputs).collect_vec();
        let func_input_infos = input_infos
            .iter()
            .copied()
            .map(|val| {
                val.func_supertype().map(|(styp, new_substs)| {
                    substs.extend(new_substs);
                    styp
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| ctx.make_error(ErrorKind::Unranked("function call")))?;

        let tir_func =
            monomorphize_and_analyze(uir_func, func_input_infos, tr.uir, tr.tir, ctx.input_spans)
                .map_err(|mut err| {
                let Error::FancyError(fancy_err) = &mut err;
                fancy_err.call_spans.push(ctx.span.clone());
                err
            })?;

        tr.tir.borrow_mut().bindings.push(tir::Binding {
            span: uir_binding.span.clone(),
            func_id: uir_binding.func_id.clone(),
            hash: uir_binding.hash,
            func: tir_func,
        });
        tir = tr.tir.borrow();

        (tir.bindings.len() - 1, tir.bindings.last().unwrap())
    };

    let mut subst_cache = substs.into_iter().collect();
    let out_infos = binding
        .func
        .meta
        .outputs
        .iter()
        .map(|val_info| val_info.instantiate_vars(&mut subst_cache))
        .collect_vec();

    let outputs = tr.add_node_dyn(
        tir::Node::Call(i),
        out_infos,
        inputs.iter().copied(),
        binding.func.outs_count(),
    );
    for (i, (out, _)) in outputs.into_iter().enumerate() {
        tr.associate((uir_node_idx, i), out);
    }

    Ok(())
}

impl ValueInfo {
    /// Attempt to match to a `ValueInfo` in the inputs of a monomorphization of a function
    ///
    /// If not matching, returns `None`.
    /// If matching, returns `Some` with a list of variable substitutions to make
    pub fn match_monomorphization(&self, func_input_info: &Self) -> Option<Vec<(usize, Expr)>> {
        use types::ArrayInfo as Ai;
        use types::ScalarInfo as S;
        match (self, func_input_info) {
            (Self::Scalar(this_val), Self::Scalar(func_val)) => match (this_val, func_val) {
                (S::Bool(this_val), S::Bool(func_val)) => func_val
                    .is_none_or(|func_val| this_val.is_some_and(|this_val| this_val == func_val)),
                (S::Int(this_val, this_inf), S::Int(func_val, func_inf)) => {
                    *func_inf
                        || !*this_inf
                            && func_val.is_none_or(|func_val| {
                                this_val.is_some_and(|this_val| this_val == func_val)
                            })
                }
                #[expect(clippy::float_cmp)]
                (S::Float(this_val), S::Float(func_val)) => func_val
                    .is_none_or(|func_val| this_val.is_some_and(|this_val| this_val == func_val)),
                (S::Char(this_val), S::Char(func_val)) => func_val
                    .is_none_or(|func_val| this_val.is_some_and(|this_val| this_val == func_val)),
                _ => false,
            }
            .then_some(Vec::new()),
            (Self::Array(this_val), Self::Array(func_val)) => match (&**this_val, &**func_val) {
                (
                    Ai::Known {
                        element_type: this_element_type,
                        value: this_value,
                    },
                    Ai::Ranked {
                        element_type: func_element_type,
                        shape: func_shape,
                    },
                ) => {
                    if this_value.shape.len() != func_shape.len() {
                        return None;
                    }
                    for (&this_ax, func_ax) in this_value.shape.iter().zip(func_shape) {
                        if func_ax.as_const() != Some(this_ax.cast_signed()) {
                            return None;
                        }
                    }
                    this_element_type.match_monomorphization(func_element_type)
                }
                (
                    Ai::Ranked {
                        element_type: this_element_type,
                        shape: this_shape,
                    },
                    Ai::Ranked {
                        element_type: func_element_type,
                        shape: func_shape,
                    },
                ) => {
                    if this_shape.len() != func_shape.len() {
                        return None;
                    }
                    let mut substs = this_element_type.match_monomorphization(func_element_type)?;
                    for (this_ax, func_ax) in this_shape.iter().zip(func_shape) {
                        if let Some(this_ax) = this_ax.as_const() {
                            match func_ax.as_const() {
                                Some(func_ax) if func_ax == this_ax => continue,
                                _ => return None,
                            }
                        }
                        let var_i = func_ax.as_single_var()?;
                        substs.push((var_i, this_ax.clone()));
                    }
                    Some(substs)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Create a supertype to use to annotate function calls
    ///
    /// Returns `None` if the input is an unranked array.
    pub fn func_supertype(&self) -> Option<(Self, Vec<(usize, Expr)>)> {
        let mut substs = Vec::new();
        let mut add_substs = |(val, new_substs)| {
            substs.extend(new_substs);
            val
        };
        let supertype = match self {
            Self::Scalar(scalar_info) => Self::Scalar(match scalar_info {
                types::ScalarInfo::Bool(_) => types::ScalarInfo::Bool(None),
                types::ScalarInfo::Int(_, inf) => types::ScalarInfo::Int(None, *inf),
                types::ScalarInfo::Float(_) => types::ScalarInfo::Float(None),
                types::ScalarInfo::Char(_) => types::ScalarInfo::Char(None),
            }),
            Self::Array(array_info) => Self::Array(Box::new(match &**array_info {
                types::ArrayInfo::Known {
                    element_type,
                    value,
                } => types::ArrayInfo::Ranked {
                    element_type: add_substs(element_type.func_supertype()?),
                    shape: value.shape.iter().copied().map(Into::into).collect(),
                },
                types::ArrayInfo::Ranked {
                    element_type,
                    shape,
                } => types::ArrayInfo::Ranked {
                    element_type: add_substs(element_type.func_supertype()?),
                    shape: shape
                        .iter()
                        .map(|ax| {
                            // Replace all unknown axes with brand new variables
                            if ax.as_const().is_some() {
                                ax.clone()
                            } else {
                                let var = Expr::new_var();
                                substs.push((var.as_single_var().unwrap(), ax.clone()));
                                var
                            }
                        })
                        .collect(),
                },
                types::ArrayInfo::Unranked { .. } => return None,
            })),
            Self::Map(map_info) => Self::Map(Box::new(types::MapInfo {
                key_type: add_substs(map_info.key_type.func_supertype()?),
                value_type: add_substs(map_info.value_type.func_supertype()?),
            })),
            Self::Struct(struct_info) => Self::Struct(types::StructInfo {
                fields: struct_info
                    .fields
                    .iter()
                    .map(|(name, typ)| Some((name.clone(), add_substs(typ.func_supertype()?))))
                    .collect::<Option<_>>()?,
            }),
            Self::Enum(enum_info) => Self::Enum(types::EnumInfo {
                variants: enum_info
                    .variants
                    .iter()
                    .map(|(name, struct_info)| {
                        Some((
                            name.clone(),
                            types::StructInfo {
                                fields: struct_info
                                    .fields
                                    .iter()
                                    .map(|(name, typ)| {
                                        Some((name.clone(), add_substs(typ.func_supertype()?)))
                                    })
                                    .collect::<Option<_>>()?,
                            },
                        ))
                    })
                    .collect::<Option<_>>()?,
            }),
        };
        Some((supertype, substs))
    }

    pub fn instantiate_vars(&self, subst_cache: &mut HashMap<usize, Expr>) -> Self {
        match self {
            Self::Array(array_info) => Self::Array(Box::new(match &**array_info {
                ai @ types::ArrayInfo::Known { .. } => ai.clone(),
                types::ArrayInfo::Ranked {
                    element_type,
                    shape,
                } => types::ArrayInfo::Ranked {
                    element_type: element_type.instantiate_vars(subst_cache),
                    shape: shape
                        .iter()
                        .map(|ax| ax.instantiate_vars(subst_cache))
                        .collect(),
                },
                types::ArrayInfo::Unranked {
                    element_type,
                    shape_prefix,
                    shape_suffix,
                } => types::ArrayInfo::Unranked {
                    element_type: element_type.instantiate_vars(subst_cache),
                    shape_prefix: shape_prefix
                        .iter()
                        .map(|ax| ax.instantiate_vars(subst_cache))
                        .collect(),
                    shape_suffix: shape_suffix
                        .iter()
                        .map(|ax| ax.instantiate_vars(subst_cache))
                        .collect(),
                },
            })),
            Self::Map(map_info) => Self::Map(Box::new(types::MapInfo {
                key_type: map_info.key_type.instantiate_vars(subst_cache),
                value_type: map_info.value_type.instantiate_vars(subst_cache),
            })),
            _ => self.clone(),
        }
    }
}
