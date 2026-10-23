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
}

impl FunctionTranslator<'_> {
    fn add_node<const N: usize>(
        &self,
        node: tir::Node,
        info: impl Into<tir::NodeMeta>,
        inputs: impl IntoIterator<Item = TirValue>,
    ) -> [(TirValue, &ValueInfo); N] {
        let mut graph = self.tir_graph.borrow_mut();
        let node_idx = graph.add_node(node);
        self.info_map.insert(node_idx, info.into());
        for (in_i, input) in inputs.into_iter().enumerate() {
            graph.add_edge(node_idx, input.node_idx, (input.out_i, in_i));
        }
        <[TirValue; N]>::try_from(
            (0..N)
                .map(|out_i| TirValue { node_idx, out_i })
                .collect_vec(),
        )
        .unwrap()
        .map(|val| (val, &self.info_map[&val.node_idx][val.out_i]))
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
            kind,
        }))
    }
    fn error<T>(&self, kind: ErrorKind) -> Result<T, Error> {
        Err(self.make_error(kind))
    }
}

pub fn construct_tir(uir: &uir::Uir) -> Result<tir::Tir, Error> {
    let mut tir = RefCell::new(tir::Tir {
        structs: translate_structs(uir)?,
        enums: Vec::new(),
        bindings: Vec::new(),
        main: None,
        spans: uir.spans.clone(),
        files: uir.files.clone(),
    });

    if let Some((uir_main, span)) = &uir.main {
        let main = monomorphize_and_analyze(uir_main, &[], uir, &tir)?;
        tir.get_mut().main = Some((main, *span));
    }

    Ok(tir.into_inner())
}

fn translate_structs(uir: &uir::Uir) -> Result<Vec<tir::Struct>, Error> {
    uir.structs
        .iter()
        .map(|struct_def| {
            tir::Struct::try_from(struct_def).map_err(|err| {
                Error::FancyError(Box::new(FancyError {
                    files: Rc::clone(&uir.files),
                    span: struct_def.fields[err.field_idx].2.clone().into(),
                    input_spans: Vec::new(),
                    kind: ErrorKind::Struct(err),
                }))
            })
        })
        .collect::<Result<_, _>>()
}

fn monomorphize_and_analyze(
    uir_func: &uir::Function,
    inputs: impl Into<Vec<ValueInfo>>,
    uir: &Uir,
    tir: &RefCell<Tir>,
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
            (
                tr.value_map.borrow()[&UirValue {
                    node_idx: e.target(),
                    out_i: e.weight().0,
                }],
                &tr.uir.spans[tr.uir_func.spans.get(&e.target()).copied().unwrap_or(0)],
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

#[derive(Debug, thiserror::Error, Clone, Copy)]
pub enum FieldError {
    #[error("Fields require shape specifiers")]
    UnspecifiedShape,
    #[error("Field's scalar type must be specified")]
    UnspecifiedScalar,
    #[error("Fields cannot contain unnamed structs")]
    UnnamedSubstruct,
}
#[derive(Debug, thiserror::Error, Clone, Copy)]
#[error("{error}")]
pub struct StructError {
    error: FieldError,
    field_idx: usize,
}
impl TryFrom<&crate::uir::Struct> for tir::Struct {
    type Error = StructError;
    fn try_from(uir_struct: &crate::uir::Struct) -> Result<Self, Self::Error> {
        let mut fields = Vec::new();
        for (field_idx, (field_name, field_type, _field_span)) in
            uir_struct.fields.iter().enumerate()
        {
            fields.push((
                field_name.clone(),
                ValueInfo::try_from(field_type.clone())
                    .map_err(|error| StructError { error, field_idx })?,
            ));
        }
        Ok(Self {
            name: uir_struct.name.clone(),
            info: types::BoundStructInfo {
                fields: fields.into(),
            },
        })
    }
}
impl TryFrom<uiua::Type> for ValueInfo {
    type Error = FieldError;
    fn try_from(value: uiua::Type) -> Result<Self, Self::Error> {
        if value.shape.is_scalar() {
            value.scalar.try_into()
        } else if value.shape.is_any() {
            Err(FieldError::UnspecifiedShape)
        } else {
            Ok(ValueInfo::Array(Box::new(types::ArrayInfo::Ranked {
                element_type: value.scalar.try_into()?,
                shape: value
                    .shape
                    .dims
                    .into_iter()
                    .map(tir::polynomial::Expr::from)
                    .collect(),
            })))
        }
    }
}
impl TryFrom<uiua::Scalar> for ValueInfo {
    type Error = FieldError;
    fn try_from(value: uiua::Scalar) -> Result<Self, Self::Error> {
        use uiua::Scalar as UType;
        use uiua::ScalarBox as UBoxType;
        Ok(match value {
            UType::Bool => ValueInfo::Scalar(types::ScalarInfo::Bool(None)),
            UType::Nat | UType::Int => ValueInfo::Scalar(types::ScalarInfo::Int(None, false)),
            UType::Num => ValueInfo::Scalar(types::ScalarInfo::Float(None)),
            UType::Ascii | UType::Char => ValueInfo::Scalar(types::ScalarInfo::Char(None)),
            UType::Box(UBoxType::Def(Some(struct_name), types)) => {
                ValueInfo::Struct(types::UnboundStructInfo {
                    name: struct_name.into(),
                    fields: types
                        .into_iter()
                        .map(ValueInfo::try_from)
                        .collect::<Result<_, _>>()?,
                })
            }
            UType::Box(UBoxType::All(internal)) => internal.unboxed().try_into()?,
            UType::Box(UBoxType::Def(None, _)) => return Err(FieldError::UnnamedSubstruct),
            UType::Any => return Err(FieldError::UnspecifiedScalar),
            _ => todo!("{value:?}"),
        })
    }
}
