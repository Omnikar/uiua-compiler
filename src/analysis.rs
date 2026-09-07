mod error;
mod impls;

use itertools::Itertools;
use petgraph::visit::EdgeRef;
use std::collections::HashMap;
use std::rc::Rc;

use crate::generic_ir::{Graph, NodeIndex};
use crate::hir::{self, Hir};
use crate::mir::{self, Mir, ValueInfo, types};
use error::{ErrorKind, FancyError};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    FancyError(FancyError),
}

pub fn construct_mir(hir: &hir::Hir) -> Result<mir::Mir, Error> {
    let mut mir = mir::Mir {
        structs: Vec::new(),
        enums: Vec::new(),
        bindings: Vec::new(),
        main: None,
        spans: hir.spans.clone(),
        files: hir.files.clone(),
    };

    if let Some((hir_main, span)) = &hir.main {
        let main = monomorphize_and_analyze(hir_main, &[], hir, &mut mir)?;
        mir.main = Some((main, *span));
    }

    Ok(mir)
}

fn monomorphize_and_analyze(
    hir_func: &hir::Function,
    inputs: &[mir::ValueInfo],
    hir: &Hir,
    mir: &mut Mir,
) -> Result<mir::Function, Error> {
    let mut graph = Graph::new();
    let input_idx = graph.add_node(mir::Node::Input);
    let mut graph_map = HashMap::new();
    graph_map.insert(hir_func.input_idx, input_idx);
    let mut info_map = HashMap::new();
    info_map.insert(input_idx, inputs.iter().cloned().collect_vec());

    for node_idx in hir_func.graph.node_indices() {
        analyze_node(
            node_idx,
            hir_func,
            &mut graph,
            &mut graph_map,
            &mut info_map,
            hir,
            mir,
        )?;
    }
    for edge_idx in hir_func.graph.edge_indices() {
        let (source, target) = hir_func.graph.edge_endpoints(edge_idx).unwrap();
        let weight = hir_func.graph[edge_idx];
        graph.add_edge(graph_map[&source], graph_map[&target], weight);
    }
    let input_idx = graph_map[&hir_func.input_idx];
    let output_idx = graph_map[&hir_func.output_idx];
    let outputs = graph
        .edges(output_idx)
        .sorted_by_key(|e| e.weight().1)
        .map(|e| info_map[&graph_map[&e.target()]][e.weight().0].clone())
        .collect_vec();
    Ok(mir::Function {
        meta: mir::FunctionMeta {
            inputs: inputs.to_owned(),
            outputs,
        },
        graph,
        input_idx,
        output_idx,
        node_metas: info_map,
        spans: hir_func
            .spans
            .iter()
            .map(|(k, v)| (graph_map[k], *v))
            .collect(),
    })
}

#[derive(Clone, Copy)]
struct AnalyzeContext<'a> {
    hir: &'a Hir,
    span: &'a uiua::Span,
    input_spans: &'a [&'a uiua::Span],
}

impl AnalyzeContext<'_> {
    fn error<T>(&self, kind: ErrorKind) -> Result<T, Error> {
        Err(Error::FancyError(FancyError {
            files: Rc::clone(&self.hir.files),
            span: self.span.clone(),
            input_spans: self.input_spans.iter().map(|&x| x.clone()).collect(),
            kind,
        }))
    }
}

fn analyze_node(
    hir_node_idx: NodeIndex,
    hir_func: &hir::Function,
    mir_graph: &mut Graph<mir::Node>,
    graph_map: &mut HashMap<NodeIndex, NodeIndex>,
    info_map: &mut HashMap<NodeIndex, mir::NodeMeta>,
    hir: &Hir,
    mir: &mut Mir,
) -> Result<(), Error> {
    let hir_node = &hir_func.graph[hir_node_idx];
    let span = &hir.spans[hir_func.spans.get(&hir_node_idx).copied().unwrap_or(0)];

    let (input_infos, input_spans): (Vec<_>, Vec<_>) = hir_func
        .graph
        .edges(hir_node_idx)
        .sorted_by_key(|e| e.weight().1)
        .map(|e| {
            (
                &info_map[&graph_map[&e.target()]][e.weight().0],
                &hir.spans[hir_func.spans.get(&e.target()).copied().unwrap_or(0)],
            )
        })
        .unzip();

    let ctx = AnalyzeContext {
        hir,
        span,
        input_spans: &input_spans,
    };

    match hir_node {
        hir::Node::Input => {
            // Input node is expected to already exist by this point; do nothing
        }
        hir::Node::Output => {
            let node_idx = mir_graph.add_node(mir::Node::Output);
            graph_map.insert(hir_node_idx, node_idx);
        }
        hir::Node::Constant(value) => {
            let value_info = mir::ValueInfo::try_from(value).map_err(|err| {
                Error::FancyError(FancyError {
                    files: hir.files.clone(),
                    span: hir.spans[hir_func.spans.get(&hir_node_idx).copied().unwrap_or(0)]
                        .clone(),
                    input_spans: Vec::new(),
                    kind: ErrorKind::UiuaValue(err),
                })
            })?;
            let node_idx = mir_graph.add_node(mir::Node::Constant(value_info.clone()));
            graph_map.insert(hir_node_idx, node_idx);
            info_map.insert(node_idx, [value_info].into());
        }
        hir::Node::FuncPrim(prim) if let Some(impl_fn) = impls::monadic_prim(*prim) => {
            let node_idx = mir_graph.add_node(mir::Node::FuncPrim(prim.into()));
            graph_map.insert(hir_node_idx, node_idx);
            info_map.insert(node_idx, [impl_fn(input_infos[0], ctx)?].into());
        }
        // hir::Node::FuncPrim(primitive) => todo!(),
        // hir::Node::FuncImplPrim(impl_primitive) => todo!(),
        // hir::Node::ModPrim(primitive, functions) => todo!(),
        // hir::Node::ModImplPrim(impl_primitive, functions) => todo!(),
        // hir::Node::Call(function) => todo!(),
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
