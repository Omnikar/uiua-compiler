use itertools::Itertools;
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

use crate::generic_ir::{Graph, NodeIndex};
use crate::hir::{self, Hir};
use crate::mir::{self, Mir, ValueInfo};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    UiuaValueError(#[from] UiuaValueError),
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
    let mut info_map = HashMap::new();
    info_map.insert(hir_func.input_idx, inputs.iter().cloned().collect_vec());
    // TODO: analyze_node in graph index order
    for node_idx in hir_func.graph.node_indices() {
        analyze_node(node_idx, hir_func, &mut info_map, hir, mir)?;
    }
    todo!()
}

fn analyze_node(
    node_idx: NodeIndex,
    hir_func: &hir::Function,
    info_map: &mut HashMap<NodeIndex, mir::NodeMeta>,
    hir: &Hir,
    mir: &mut Mir,
) -> Result<(), Error> {
    let node = dbg!(&hir_func.graph[node_idx]);

    let input_infos = hir_func
        .graph
        .edges(node_idx)
        .sorted_by_key(|e| e.weight().1)
        .map(|e| &info_map[&e.target()][e.weight().0])
        .collect_vec();

    match node {
        hir::Node::Input => return Ok(()),
        hir::Node::Output => {
            todo!("Idk");
        }
        hir::Node::Constant(value) => {
            let value_info = mir::ValueInfo::try_from(value)?;
            info_map.insert(node_idx, [value_info].into());
        }
        hir::Node::FuncPrim(primitive) => todo!(),
        hir::Node::FuncImplPrim(impl_primitive) => todo!(),
        hir::Node::ModPrim(primitive, functions) => todo!(),
        hir::Node::ModImplPrim(impl_primitive, functions) => todo!(),
        hir::Node::Call(function) => todo!(),
    }

    todo!()
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
