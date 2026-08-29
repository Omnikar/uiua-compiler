use itertools::Itertools;
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize, de::Error};
use std::collections::HashMap;
use std::path::PathBuf;

pub use petgraph::stable_graph::{NodeIndex, StableDiGraph as Graph};

// TODO: Name this
#[derive(Debug, Serialize, Deserialize)]
pub struct Hir {
    pub datadefs: Vec<Datadef>,
    pub bindings: Vec<Binding>,
    pub spans: Vec<uiua::Span>,
    pub files: HashMap<PathBuf, String>,
}

impl std::fmt::Display for Hir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ron = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new()).unwrap();

        // This flattens 1D lists of numbers
        let re = regex::Regex::new(r"\[\s*(?:\d+\s*,\s*)*\d*\s*\]").unwrap();
        let mut ron = re
            .replace_all(&ron, |caps: &regex::Captures| {
                let content = &caps[0];
                let mut s = String::new();
                for c in content.chars() {
                    if !c.is_whitespace() {
                        s.push(c);
                        if c == ',' {
                            s.push(' ');
                        }
                    }
                }
                s
            })
            .into_owned();
        ron = ron.replace(", ]", "]");
        write!(f, "{ron}")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Binding {
    pub span: uiua::CodeSpan,
    pub func_id: uiua::FunctionId,
    pub hash: u64,
    pub func: Function,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Datadef {
    pub name: String,

    // We store type annotations as raw Uiua values for now,
    // they will be interpreted later.
    pub fields: Vec<uiua::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Node {
    Input,
    Output,
    // Should this store a custom type instead?
    Constant(uiua::Value),
    FuncPrim(uiua::Primitive),
    FuncImplPrim(uiua::ImplPrimitive),
    ModPrim(uiua::Primitive, Vec<Function>),
    ModImplPrim(uiua::ImplPrimitive, Vec<Function>),
}

#[derive(Debug, Clone)]
pub struct Function {
    pub graph: Graph<Node, (usize, usize)>,
    pub input_idx: NodeIndex,
    pub output_idx: NodeIndex,
    pub spans: HashMap<NodeIndex, usize>,
}
impl Function {
    /// The number of inputs to this function
    pub fn args_count(&self) -> usize {
        self.graph
            .edges_directed(self.input_idx, petgraph::Direction::Incoming)
            .map(|e| e.weight().0)
            .max()
            .map_or(0, |x| x + 1)
    }
    /// The number of outputs from this function
    pub fn outs_count(&self) -> usize {
        self.graph
            .edges(self.output_idx)
            .map(|e| e.weight().1)
            .max()
            .map_or(0, |x| x + 1)
    }
}

type FunctionSerializable = Vec<(Vec<usize>, Node, Vec<usize>, usize)>;
impl From<&Function> for FunctionSerializable {
    fn from(func: &Function) -> Self {
        let mut toposort = petgraph::algo::toposort(&func.graph, None)
            .expect("Compiler generated a cyclic data flow graph. This is a bug.");
        assert_eq!(toposort[0], func.output_idx);
        toposort.reverse();
        assert_eq!(toposort[0], func.input_idx);

        let mut ident_count = 0;
        let mut idents = HashMap::<(NodeIndex, usize), usize>::new();

        let mut vec = Vec::new();

        for node_idx in toposort {
            let inputs = func
                .graph
                .edges(node_idx)
                .map(|e| {
                    let source_node_idx = e.target();
                    let (out_i, in_i) = *e.weight();
                    (in_i, idents[&(source_node_idx, out_i)])
                })
                .sorted_by_key(|(in_i, _)| *in_i)
                .map(|(_, x)| x)
                .collect_vec();

            let output_count = func
                .graph
                .edges_directed(node_idx, petgraph::Direction::Incoming)
                .map(|e| e.weight().0)
                .max()
                .map_or(0, |x| x + 1);
            let mut outputs = Vec::new();
            for i in 0..output_count {
                let ident = ident_count;
                outputs.push(ident);
                idents.insert((node_idx, i), ident);
                ident_count += 1;
            }

            let node = func.graph[node_idx].clone();

            vec.push((
                outputs,
                node,
                inputs,
                func.spans.get(&node_idx).copied().unwrap_or(0),
            ));
        }

        vec
    }
}
#[derive(thiserror::Error, Debug)]
pub enum FunctionDeserializeError {
    #[error("Function definition missing Input line")]
    MissingInput,
    #[error("Function definition missing Output line")]
    MissingOutupt,
    #[error("Unknown identifier in function definition: {0}")]
    UnknownIdent(usize),
}
impl TryFrom<FunctionSerializable> for Function {
    type Error = FunctionDeserializeError;
    fn try_from(data: FunctionSerializable) -> Result<Self, Self::Error> {
        let mut graph = Graph::<Node, (usize, usize)>::new();
        let mut spans = HashMap::<NodeIndex, usize>::new();

        let mut idents = HashMap::<usize, (NodeIndex, usize)>::new();

        let mut instrs = data.into_iter();
        let input_idents = instrs
            .next()
            .and_then(|(idents, node, _, _)| matches!(node, Node::Input).then_some(idents))
            .ok_or(FunctionDeserializeError::MissingInput)?;

        let input_idx = graph.add_node(Node::Input);
        for (out_i, ident) in input_idents.into_iter().enumerate() {
            idents.insert(ident, (input_idx, out_i));
        }

        for (out_idents, node, in_idents, span) in instrs {
            let is_output = matches!(node, Node::Output);

            let new_node_idx = graph.add_node(node);
            spans.insert(new_node_idx, span);
            for (in_i, in_ident) in in_idents.into_iter().enumerate() {
                let (source_node_idx, out_i) = *idents
                    .get(&in_ident)
                    .ok_or(FunctionDeserializeError::UnknownIdent(in_ident))?;
                graph.add_edge(new_node_idx, source_node_idx, (out_i, in_i));
            }
            for (out_i, out_ident) in out_idents.into_iter().enumerate() {
                idents.insert(out_ident, (new_node_idx, out_i));
            }

            if is_output {
                return Ok(Self {
                    graph,
                    input_idx,
                    output_idx: new_node_idx,
                    spans,
                });
            }
        }

        Err(FunctionDeserializeError::MissingOutupt)
    }
}

impl Serialize for Function {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        FunctionSerializable::from(self).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Function {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        FunctionSerializable::deserialize(deserializer)
            .and_then(|x| Function::try_from(x).map_err(D::Error::custom))
    }
}
