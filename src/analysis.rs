use itertools::Itertools;
use petgraph::visit::EdgeRef;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::generic_ir::{Graph, NodeIndex};
use crate::hir::{self, Hir};
use crate::mir::{self, Mir, ValueInfo};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    FancyError(FancyError),
}

#[derive(thiserror::Error, Debug)]
#[error("{0}: {1}", .span, .kind)]
pub struct FancyError {
    files: Rc<HashMap<PathBuf, String>>,
    span: uiua::Span,
    input_spans: Vec<uiua::Span>,
    kind: ErrorKind,
}

#[derive(thiserror::Error, Debug)]
pub enum ErrorKind {
    #[error("{0}")]
    UiuaValue(UiuaValueError),
    #[error("Cannot take the square root of a character")]
    SqrtChar,
}

impl FancyError {
    pub fn eprint(&self) {
        use ariadne::{ColorGenerator, Label, Report, ReportKind, Source};

        let mut colors = ColorGenerator::new();

        match &self.kind {
            ErrorKind::UiuaValue(err) => {
                let (source_path, source, range) = span_to_ariadne(&self.span, &self.files);
                Report::build(ReportKind::Error, (&source_path, range.clone()))
                    .with_message(err)
                    .with_label(
                        Label::new((&source_path, range.clone()))
                            .with_message(err)
                            .with_color(colors.next()),
                    )
                    .finish()
                    .eprint((&source_path, Source::from(source)))
                    .unwrap();
            }
            ErrorKind::SqrtChar => {
                let (source_path, source, range) = span_to_ariadne(&self.span, &self.files);
                let (input_source_path, _input_source, input_range) =
                    span_to_ariadne(&self.input_spans[0], &self.files);

                Report::build(ReportKind::Error, (&source_path, range.clone()))
                    .with_message(&self.kind)
                    .with_label(
                        Label::new((&source_path, range))
                            .with_message("Square root expects numbers")
                            .with_color(colors.next()),
                    )
                    .with_label(
                        Label::new((&input_source_path, input_range))
                            .with_message("Characters produced here")
                            .with_color(colors.next())
                            .with_order(-1),
                    )
                    .finish()
                    .eprint((&source_path, Source::from(source)))
                    .unwrap();
            }
        }
    }
}

fn span_to_ariadne<'a>(
    span: &'a uiua::Span,
    files: &'a HashMap<PathBuf, String>,
) -> (std::borrow::Cow<'a, str>, &'a str, std::ops::Range<usize>) {
    span.code_ref()
        .and_then(|span| code_span_to_ariadne(span, files))
        .unwrap_or_else(|| (std::borrow::Cow::default(), "", 0..0))
}

fn code_span_to_ariadne<'a>(
    span: &'a uiua::CodeSpan,
    files: &'a HashMap<PathBuf, String>,
) -> Option<(std::borrow::Cow<'a, str>, &'a str, std::ops::Range<usize>)> {
    match &span.src {
        uiua::InputSrc::File(path) => Some((
            path.to_string_lossy(),
            &*files[&**path],
            span.start.char_pos as usize..span.end.char_pos as usize,
        )),
        uiua::InputSrc::Str(_) => None,
        uiua::InputSrc::Macro(code_span) => code_span_to_ariadne(code_span, files),
        uiua::InputSrc::Literal(string) => Some((
            std::borrow::Cow::default(),
            string,
            0..string.chars().count(),
        )),
    }
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

    match hir_node {
        hir::Node::Input => {}
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
        hir::Node::FuncPrim(uiua::Primitive::Sqrt) => {
            let node_idx = mir_graph.add_node(mir::Node::FuncPrim(uiua::Primitive::Sqrt));
            graph_map.insert(hir_node_idx, node_idx);

            let input_info = input_infos[0];
            info_map.insert(
                node_idx,
                [analyze_sqrt(input_info, hir, span, &input_spans)?].into(),
            );
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

fn analyze_sqrt(
    input_info: &ValueInfo,
    hir: &Hir,
    span: &uiua::Span,
    input_spans: &[&uiua::Span],
) -> Result<ValueInfo, Error> {
    Ok(match input_info {
        ValueInfo::Bool(_) => input_info.clone(),
        #[allow(clippy::cast_precision_loss)]
        ValueInfo::Int(i) => ValueInfo::Float(i.map(|i| (i as f64).sqrt())),
        ValueInfo::Float(f) => ValueInfo::Float(f.map(f64::sqrt)),
        ValueInfo::Char(_) => {
            return Err(Error::FancyError(FancyError {
                files: Rc::clone(&hir.files),
                span: span.clone(),
                input_spans: input_spans.iter().map(|&x| x.clone()).collect(),
                kind: ErrorKind::SqrtChar,
            }));
        }
        ValueInfo::Array(array_info) => match &**array_info {
            mir::types::ArrayInfo::Known { scalar_type, value } => {
                let scalar_type = analyze_sqrt(scalar_type, hir, span, input_spans)?;
                let value = mir::types::ArrayValue {
                    shape: value.shape.clone(),
                    data: value
                        .data
                        .iter()
                        .map(|x| analyze_sqrt(x, hir, span, input_spans))
                        .collect::<Result<Vec<_>, _>>()?,
                };
                ValueInfo::Array(Box::new(mir::types::ArrayInfo::Known {
                    scalar_type,
                    value,
                }))
            }
            mir::types::ArrayInfo::Ranked { scalar_type, shape } => {
                let scalar_type = analyze_sqrt(scalar_type, hir, span, input_spans)?;
                ValueInfo::Array(Box::new(mir::types::ArrayInfo::Ranked {
                    scalar_type,
                    shape: shape.clone(),
                }))
            }
            mir::types::ArrayInfo::Unranked {
                scalar_type,
                shape_prefix,
                shape_suffix,
            } => {
                let scalar_type = analyze_sqrt(scalar_type, hir, span, input_spans)?;
                ValueInfo::Array(Box::new(mir::types::ArrayInfo::Unranked {
                    scalar_type,
                    shape_prefix: shape_prefix.clone(),
                    shape_suffix: shape_suffix.clone(),
                }))
            }
        },
        _ => todo!(),
    })
}

#[derive(thiserror::Error, Debug)]
pub enum UiuaValueError {
    #[error("Could not infer array type")]
    NoArrayType,
}

impl TryFrom<&uiua::Value> for ValueInfo {
    type Error = UiuaValueError;
    fn try_from(value: &uiua::Value) -> Result<Self, Self::Error> {
        Ok(if value.rank() == 0 {
            match value {
                uiua::Value::Byte(array) => match *array.elements().next().unwrap() {
                    b @ (0 | 1) => Self::Bool(Some(b != 0)),
                    i => Self::Int(Some(i.into())),
                },
                uiua::Value::Num(array) => match *array.elements().next().unwrap() {
                    b @ (0.0 | 1.0) => Self::Bool(Some(b != 0.0)),
                    // f if let Ok(i) = i64::try_from(f) => Self::Int(Some(i)),
                    f => Self::Float(Some(f)),
                },
                uiua::Value::Char(array) => Self::Char(Some(*array.elements().next().unwrap())),
                uiua::Value::Box(array) => {
                    let val = array.elements().next().unwrap();
                    Self::try_from(&val.0)?
                }
                uiua::Value::Complex(_array) => {
                    unimplemented!("Complex numbers are currently not supported")
                }
                uiua::Value::Mv(_array) => {
                    unimplemented!("Multivectors are currently not supported")
                }
            }
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
            Self::Array(Box::new(mir::types::ArrayInfo::Known {
                scalar_type,
                value: mir::types::ArrayValue { shape, data },
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
