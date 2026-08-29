use itertools::Itertools;
use petgraph::visit::EdgeRef;
use serde::{
    Deserialize, Serialize,
    ser::{SerializeSeq, SerializeTuple, SerializeTupleStruct},
};
use std::cell::{Cell, RefCell};
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

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug)]
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

impl Serialize for Function {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut toposort = petgraph::algo::toposort(&self.graph, None)
            .expect("Compiler generated a cyclic data flow graph. This is a bug.");
        assert_eq!(toposort[0], self.output_idx);
        toposort.reverse();
        assert_eq!(toposort[0], self.input_idx);

        let ident_count = Cell::from(0);
        let idents = RefCell::from(HashMap::<(NodeIndex, usize), usize>::new());

        let mut seq = serializer.serialize_seq(Some(toposort.len()))?;

        struct Operation<'a> {
            node_idx: NodeIndex,
            span: usize,
            graph: &'a Graph<Node, (usize, usize)>,
            ident_count: &'a Cell<usize>,
            idents: &'a RefCell<HashMap<(NodeIndex, usize), usize>>,
        }
        impl Serialize for Operation<'_> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                let inputs = self
                    .graph
                    .edges(self.node_idx)
                    .map(|e| {
                        let source_node_idx = e.target();
                        let (out_i, in_i) = *e.weight();
                        (in_i, self.idents.borrow()[&(source_node_idx, out_i)])
                    })
                    .sorted_by_key(|(in_i, _)| *in_i)
                    .map(|(_, x)| x)
                    .collect_vec();

                let output_count = self
                    .graph
                    .edges_directed(self.node_idx, petgraph::Direction::Incoming)
                    .map(|e| e.weight().0)
                    .max()
                    .map_or(0, |x| x + 1);
                let mut outputs = Vec::new();
                for i in 0..output_count {
                    let ident = self.ident_count.get();
                    outputs.push(ident);
                    self.idents.borrow_mut().insert((self.node_idx, i), ident);
                    self.ident_count.update(|x| x + 1);
                }

                let node = &self.graph[self.node_idx];

                let mut assign = serializer.serialize_tuple(4)?;
                assign.serialize_element(&outputs)?;
                assign.serialize_element(&node)?;
                assign.serialize_element(&inputs)?;
                assign.serialize_element(&self.span)?;

                assign.end()
            }
        }

        for node_idx in toposort {
            seq.serialize_element(&Operation {
                node_idx,
                span: self.spans.get(&node_idx).copied().unwrap_or(0),
                graph: &self.graph,
                ident_count: &ident_count,
                idents: &idents,
            })?;
        }

        seq.end()
    }
}
impl<'de> Deserialize<'de> for Function {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        todo!()
    }
}
