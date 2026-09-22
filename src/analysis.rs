mod error;
mod impls;

use derive_more::From;
use elsa::map::FrozenMap;
use itertools::Itertools;
use petgraph::visit::EdgeRef;
use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::generic_ir::{FunctionNode, Graph, NodeIndex};
use crate::tir::{self, Tir, ValueInfo, types};
use crate::uir::{self, Uir};
use error::{ErrorKind, FancyError};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    FancyError(Box<FancyError>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
struct UirValue {
    node_idx: NodeIndex,
    out_i: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
struct TirValue {
    node_idx: NodeIndex,
    out_i: usize,
}

/// Responsible for managing the translation of UIR to TIR
#[derive(Clone)]
struct FunctionTranslator<'ctx> {
    uir: &'ctx Uir,
    uir_func: &'ctx uir::Function,
    tir: &'ctx RefCell<Tir>,
    tir_graph: RefCell<Graph<tir::Node>>,
    value_map: RefCell<HashMap<UirValue, TirValue>>,
    info_map: FrozenMap<NodeIndex, tir::NodeMeta>,
    span_map: RefCell<HashMap<NodeIndex, usize>>,
    func_input_spans: Vec<&'ctx uiua::Span>,
}

impl FunctionTranslator<'_> {
    fn add_node<const N: usize>(
        &self,
        node: tir::Node,
        info: impl Into<tir::NodeMeta>,
        inputs: impl IntoIterator<Item = TirValue>,
    ) -> [(TirValue, &ValueInfo); N] {
        self.add_node_dyn(node, info, inputs, N).try_into().unwrap()
    }

    fn add_node_dyn(
        &self,
        node: tir::Node,
        info: impl Into<tir::NodeMeta>,
        inputs: impl IntoIterator<Item = TirValue>,
        n: usize,
    ) -> Vec<(TirValue, &ValueInfo)> {
        let mut graph = self.tir_graph.borrow_mut();
        let node_idx = graph.add_node(node);
        self.info_map.insert(node_idx, info.into());
        for (in_i, input) in inputs.into_iter().enumerate() {
            graph.add_edge(node_idx, input.node_idx, (input.out_i, in_i));
        }
        (0..n)
            .map(|out_i| TirValue { node_idx, out_i })
            .map(|val| (val, &self.info_map[&val.node_idx][val.out_i]))
            .collect()
    }

    fn associate(&self, from: impl Into<UirValue>, to: impl Into<TirValue>) {
        let from = from.into();
        let to = to.into();
        self.value_map.borrow_mut().insert(from, to);

        let span = self.uir_func.spans[&from.node_idx];
        self.span_map.borrow_mut().insert(to.node_idx, span);
    }

    fn infos<const N: usize>(&self, values: [TirValue; N]) -> [&ValueInfo; N] {
        values.map(|val| &self.info_map[&val.node_idx][val.out_i])
    }

    fn infos_dyn(
        &self,
        values: impl IntoIterator<Item = impl Borrow<TirValue>>,
    ) -> impl Iterator<Item = &ValueInfo> {
        values.into_iter().map(|val| {
            let val = val.borrow();
            &self.info_map[&val.node_idx][val.out_i]
        })
    }

    fn get_uir_span(&self, uir_value: UirValue) -> &uiua::Span {
        if self.uir_func.graph[uir_value.node_idx] == uir::Node::Input {
            self.func_input_spans[uir_value.out_i]
        } else {
            &self.uir.spans[self
                .uir_func
                .spans
                .get(&uir_value.node_idx)
                .copied()
                .unwrap_or(0)]
        }
    }
}

#[derive(Clone, Copy)]
struct AnalyzeContext<'ctx> {
    span: &'ctx uiua::Span,
    input_spans: &'ctx [&'ctx uiua::Span],
    tr: &'ctx FunctionTranslator<'ctx>,
}

impl AnalyzeContext<'_> {
    fn make_error(&self, kind: ErrorKind) -> Error {
        Error::FancyError(Box::new(FancyError {
            files: Rc::clone(&self.tr.uir.files),
            span: self.span.clone(),
            input_spans: self.input_spans.iter().map(|&x| x.clone()).collect(),
            call_spans: Vec::new(),
            kind,
        }))
    }
    fn error<T>(&self, kind: ErrorKind) -> Result<T, Error> {
        Err(self.make_error(kind))
    }
}

pub fn construct_tir(uir: &uir::Uir) -> Result<tir::Tir, Error> {
    let mut tir = RefCell::new(tir::Tir {
        structs: Vec::new(),
        enums: Vec::new(),
        bindings: Vec::new(),
        main: None,
        spans: uir.spans.clone(),
        files: uir.files.clone(),
    });

    if let Some((uir_main, span)) = &uir.main {
        let main = monomorphize_and_analyze(uir_main, &[], uir, &tir, [])?;
        tir.get_mut().main = Some((main, *span));
    }

    Ok(tir.into_inner())
}

fn monomorphize_and_analyze<'ctx>(
    uir_func: &uir::Function,
    inputs: impl Into<Vec<ValueInfo>>,
    uir: &Uir,
    tir: &RefCell<Tir>,
    func_input_spans: impl Into<Vec<&'ctx uiua::Span>>,
) -> Result<tir::Function, Error> {
    let inputs = inputs.into();

    let mut graph = Graph::new();
    let input_idx = graph.add_node(tir::Node::Input);
    let mut value_map = HashMap::new();
    for i in 0..inputs.len() {
        value_map.insert(
            UirValue {
                node_idx: input_idx,
                out_i: i,
            },
            TirValue {
                node_idx: input_idx,
                out_i: i,
            },
        );
    }
    let info_map = FrozenMap::new();
    info_map.insert(input_idx, inputs.clone());

    let translation = FunctionTranslator {
        uir,
        uir_func,
        tir,
        tir_graph: RefCell::new(graph),
        value_map: RefCell::new(value_map),
        info_map,
        span_map: RefCell::new(HashMap::new()),
        func_input_spans: func_input_spans.into(),
    };

    for node_idx in uir_func.graph.node_indices() {
        translate_node(node_idx, &translation)?;
    }

    let graph = translation.tir_graph.into_inner();
    let info_map = translation.info_map;

    let output_idx = graph
        .node_indices()
        .find(|&idx| graph[idx].is_output())
        .unwrap();
    let outputs = graph
        .edges(output_idx)
        .sorted_by_key(|e| e.weight().1)
        .map(|e| info_map[&e.target()][e.weight().0].clone())
        .collect_vec();

    Ok(tir::Function {
        meta: tir::FunctionMeta { inputs, outputs },
        graph,
        input_idx,
        output_idx,
        node_metas: info_map.into_map(),
        spans: translation.span_map.into_inner(),
    })
}

fn translate_node(uir_node_idx: NodeIndex, tr: &FunctionTranslator) -> Result<(), Error> {
    let uir_node = &tr.uir_func.graph[uir_node_idx];
    let span = &tr.uir.spans[tr.uir_func.spans.get(&uir_node_idx).copied().unwrap_or(0)];

    let (inputs, input_spans): (Vec<_>, Vec<_>) = tr
        .uir_func
        .graph
        .edges(uir_node_idx)
        .sorted_by_key(|e| e.weight().1)
        .map(|e| {
            let uir_value = UirValue {
                node_idx: e.target(),
                out_i: e.weight().0,
            };
            (
                tr.value_map.borrow()[&uir_value],
                tr.get_uir_span(uir_value),
            )
        })
        .unzip();

    let ctx = AnalyzeContext {
        span,
        input_spans: &input_spans,
        tr,
    };

    match uir_node {
        uir::Node::Input => {
            // Input node is expected to already exist by this point; do nothing
        }
        uir::Node::Output => {
            let [] = tr.add_node(tir::Node::Output, [], inputs);
        }
        uir::Node::Constant(value) => {
            let value_info = match tir::ValueInfo::try_from(value) {
                Ok(v) => v,
                Err(err) => ctx.error(ErrorKind::UiuaValue(err))?,
            };
            let [(value, _)] =
                tr.add_node(tir::Node::Constant(value_info.clone()), [value_info], []);
            tr.associate((uir_node_idx, 0), value);
        }
        uir::Node::Call(uiua_func) => {
            translate_function_call(uiua_func, &inputs, ctx, uir_node_idx, tr)?;
        }
        uir::Node::FuncPrim(prim) if let Some(impl_fn) = impls::monadic_prim(*prim) => {
            let [input_info] = tr.infos([inputs[0]]);
            let output_info = impl_fn(input_info, ctx)?;
            let [(output, _)] =
                tr.add_node(tir::Node::FuncPrim(prim.into()), [output_info], inputs);
            tr.associate((uir_node_idx, 0), output);
        }
        uir::Node::FuncPrim(prim) if let Some(impl_fn) = impls::dyadic_prim(*prim) => {
            let [lhs, rhs] = inputs.try_into().unwrap();
            let output = impl_fn(lhs, rhs, tr, ctx)?;
            tr.associate((uir_node_idx, 0), output);
        }
        _ => todo!("{uir_node:?}"),
    }

    Ok(())
}

fn translate_function_call(
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
        .borrow()
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

#[derive(thiserror::Error, Debug, Clone, Copy)]
pub enum UiuaValueError {
    #[error("Could not infer array type")]
    NoArrayType,
}

impl TryFrom<&uiua::Value> for ValueInfo {
    type Error = UiuaValueError;
    fn try_from(value: &uiua::Value) -> Result<Self, Self::Error> {
        Ok(if value.rank() == 0 {
            Self::Scalar(match value {
                uiua::Value::Byte(array) => match *array.elements().next().unwrap() {
                    b @ (0 | 1) => types::ScalarInfo::Bool(Some(b != 0)),
                    i => types::ScalarInfo::Int(Some(i.into()), false),
                },
                uiua::Value::Num(array) => match *array.elements().next().unwrap() {
                    b @ (0.0 | 1.0) => types::ScalarInfo::Bool(Some(b != 0.0)),
                    f => {
                        if f.fract() == 0.0 && f.is_finite() && f.abs() < 2.0f64.powi(53) {
                            #[expect(clippy::cast_possible_truncation)]
                            types::ScalarInfo::Int(Some(f as i64), false)
                        } else if f == f64::INFINITY {
                            types::ScalarInfo::Int(Some(i64::MAX), true)
                        } else if f == f64::NEG_INFINITY {
                            types::ScalarInfo::Int(Some(i64::MIN + 1), true)
                        } else {
                            types::ScalarInfo::Float(Some(f))
                        }
                    }
                },
                uiua::Value::Char(array) => {
                    types::ScalarInfo::Char(Some(*array.elements().next().unwrap()))
                }
                uiua::Value::Box(array) => {
                    let val = array.elements().next().unwrap();
                    return Self::try_from(&val.0);
                }
                uiua::Value::Complex(_array) => {
                    unimplemented!("Complex numbers are currently not supported")
                }
            })
        } else {
            let shape = value.shape.iter().copied().collect_vec();
            let data = value
                .elements()
                .map(Self::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            let element_type = data
                .iter()
                .cloned()
                .map(Some)
                .reduce(|x, y| x?.supertype(&y?))
                .flatten()
                .ok_or(UiuaValueError::NoArrayType)?;
            let mut val = Self::Array(Box::new(types::ArrayInfo::Known {
                element_type,
                value: types::ArrayValue { shape, data },
            }));
            if let Some(scalar_type) = val.scalar_type() {
                val.upcast_scalars(scalar_type);
            }
            val
        })
    }
}
impl TryFrom<uiua::Value> for ValueInfo {
    type Error = UiuaValueError;
    fn try_from(value: uiua::Value) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}
