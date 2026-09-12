mod error;
mod impls;

use derive_more::From;
use elsa::map::FrozenMap;
use itertools::Itertools;
use petgraph::visit::EdgeRef;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::generic_ir::{FunctionNode, Graph, NodeIndex};
use crate::hir::{self, Hir};
use crate::mir::{self, Mir, ValueInfo, types};
use error::{ErrorKind, FancyError};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    FancyError(Box<FancyError>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
struct HirValue {
    node_idx: NodeIndex,
    out_i: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
struct MirValue {
    node_idx: NodeIndex,
    out_i: usize,
}

#[derive(Clone)]
struct FunctionTranslation<'hir, 'mir> {
    hir: &'hir Hir,
    hir_func: &'hir hir::Function,
    mir: &'mir RefCell<Mir>,
    mir_graph: RefCell<Graph<mir::Node>>,
    value_map: RefCell<HashMap<HirValue, MirValue>>,
    info_map: FrozenMap<NodeIndex, mir::NodeMeta>,
    span_map: RefCell<HashMap<NodeIndex, usize>>,
}

impl FunctionTranslation<'_, '_> {
    fn add_node<const N: usize>(
        &self,
        node: mir::Node,
        info: impl Into<mir::NodeMeta>,
        inputs: impl IntoIterator<Item = MirValue>,
    ) -> [MirValue; N] {
        let mut graph = self.mir_graph.borrow_mut();
        let node_idx = graph.add_node(node);
        self.info_map.insert(node_idx, info.into());
        for (in_i, input) in inputs.into_iter().enumerate() {
            graph.add_edge(node_idx, input.node_idx, (input.out_i, in_i));
        }
        (0..N)
            .map(|out_i| MirValue { node_idx, out_i })
            .collect_vec()
            .try_into()
            .unwrap()
    }

    fn associate(&self, from: impl Into<HirValue>, to: impl Into<MirValue>) {
        let from = from.into();
        let to = to.into();
        self.value_map.borrow_mut().insert(from, to);

        let span = self.hir_func.spans[&from.node_idx];
        self.span_map.borrow_mut().insert(to.node_idx, span);
    }

    fn infos<const N: usize>(&self, values: [MirValue; N]) -> [&ValueInfo; N] {
        values.map(|val| &self.info_map[&val.node_idx][val.out_i])
    }
}

#[derive(Clone, Copy)]
struct AnalyzeContext<'ctx> {
    span: &'ctx uiua::Span,
    input_spans: &'ctx [&'ctx uiua::Span],
    tr: &'ctx FunctionTranslation<'ctx, 'ctx>,
}

impl AnalyzeContext<'_> {
    fn error<T>(&self, kind: ErrorKind) -> Result<T, Error> {
        Err(Error::FancyError(Box::new(FancyError {
            files: Rc::clone(&self.tr.hir.files),
            span: self.span.clone(),
            input_spans: self.input_spans.iter().map(|&x| x.clone()).collect(),
            kind,
        })))
    }
}

pub fn construct_mir(hir: &hir::Hir) -> Result<mir::Mir, Error> {
    let mut mir = RefCell::new(mir::Mir {
        structs: Vec::new(),
        enums: Vec::new(),
        bindings: Vec::new(),
        main: None,
        spans: hir.spans.clone(),
        files: hir.files.clone(),
    });

    if let Some((hir_main, span)) = &hir.main {
        let main = monomorphize_and_analyze(hir_main, &[], hir, &mir)?;
        mir.get_mut().main = Some((main, *span));
    }

    Ok(mir.into_inner())
}

fn monomorphize_and_analyze(
    hir_func: &hir::Function,
    inputs: impl Into<Vec<ValueInfo>>,
    hir: &Hir,
    mir: &RefCell<Mir>,
) -> Result<mir::Function, Error> {
    let inputs = inputs.into();

    let mut graph = Graph::new();
    let input_idx = graph.add_node(mir::Node::Input);
    let mut value_map = HashMap::new();
    for i in 0..inputs.len() {
        value_map.insert(
            HirValue {
                node_idx: input_idx,
                out_i: i,
            },
            MirValue {
                node_idx: input_idx,
                out_i: i,
            },
        );
    }
    let info_map = FrozenMap::new();
    info_map.insert(input_idx, inputs.clone());

    let translation = FunctionTranslation {
        hir,
        hir_func,
        mir,
        mir_graph: RefCell::new(graph),
        value_map: RefCell::new(value_map),
        info_map,
        span_map: RefCell::new(HashMap::new()),
    };

    for node_idx in hir_func.graph.node_indices() {
        translate_node(node_idx, &translation)?;
    }

    let graph = translation.mir_graph.into_inner();
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

    Ok(mir::Function {
        meta: mir::FunctionMeta { inputs, outputs },
        graph,
        input_idx,
        output_idx,
        node_metas: info_map.into_map(),
        spans: translation.span_map.into_inner(),
    })
}

fn translate_node(hir_node_idx: NodeIndex, tr: &FunctionTranslation) -> Result<(), Error> {
    let hir_node = &tr.hir_func.graph[hir_node_idx];
    let span = &tr.hir.spans[tr.hir_func.spans.get(&hir_node_idx).copied().unwrap_or(0)];

    let (inputs, input_spans): (Vec<_>, Vec<_>) = tr
        .hir_func
        .graph
        .edges(hir_node_idx)
        .sorted_by_key(|e| e.weight().1)
        .map(|e| {
            (
                tr.value_map.borrow()[&HirValue {
                    node_idx: e.target(),
                    out_i: e.weight().0,
                }],
                &tr.hir.spans[tr.hir_func.spans.get(&e.target()).copied().unwrap_or(0)],
            )
        })
        .unzip();

    let ctx = AnalyzeContext {
        span,
        input_spans: &input_spans,
        tr,
    };

    match hir_node {
        hir::Node::Input => {
            // Input node is expected to already exist by this point; do nothing
        }
        hir::Node::Output => {
            let [] = tr.add_node(mir::Node::Output, [], inputs);
        }
        hir::Node::Constant(value) => {
            let value_info = match mir::ValueInfo::try_from(value) {
                Ok(v) => v,
                Err(err) => ctx.error(ErrorKind::UiuaValue(err))?,
            };
            let [value] = tr.add_node(mir::Node::Constant(value_info.clone()), [value_info], []);
            tr.associate((hir_node_idx, 0), value);
        }
        hir::Node::FuncPrim(prim) if let Some(impl_fn) = impls::monadic_prim(*prim) => {
            let [input_info] = tr.infos([inputs[0]]);
            let output_info = impl_fn(input_info, ctx)?;
            let [output] = tr.add_node(mir::Node::FuncPrim(prim.into()), [output_info], inputs);
            tr.associate((hir_node_idx, 0), output);
        }
        _ => todo!("{hir_node:?}"),
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
                    i => types::ScalarInfo::Int(Some(i.into())),
                },
                uiua::Value::Num(array) => match *array.elements().next().unwrap() {
                    b @ (0.0 | 1.0) => types::ScalarInfo::Bool(Some(b != 0.0)),
                    f => {
                        if f.fract() == 0.0 && f.is_finite() && f.abs() < 2.0f64.powi(53) {
                            #[allow(clippy::cast_possible_truncation)]
                            types::ScalarInfo::Int(Some(f as i64))
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
                uiua::Value::Mv(_array) => {
                    unimplemented!("Multivectors are currently not supported")
                }
            })
        } else {
            let shape = value.shape.iter().copied().collect_vec();
            let data = value
                .elements()
                .map(Self::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            let scalar_type = data
                .iter()
                .cloned()
                .map(Some)
                .reduce(|x, y| x?.supertype(&y?))
                .flatten()
                .ok_or(UiuaValueError::NoArrayType)?;
            Self::Array(Box::new(types::ArrayInfo::Known {
                scalar_type,
                value: types::ArrayValue { shape, data },
            }))
        })
    }
}
impl TryFrom<uiua::Value> for ValueInfo {
    type Error = UiuaValueError;
    fn try_from(value: uiua::Value) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}
