use std::collections::HashMap;

pub use petgraph::stable_graph::{NodeIndex, StableDiGraph as Graph};

#[derive(Debug)]
pub enum Idk {}

// TODO: Name this
#[derive(Debug)]
pub struct Hir {
    pub spans: Vec<uiua::Span>,
    pub datadefs: Vec<Idk>,
    pub bindings: Vec<Binding>,
}

#[derive(Debug)]
pub struct Binding {
    pub span: uiua::CodeSpan,
    pub func_id: uiua::FunctionId,
    pub hash: u64,
    pub func: Function,
}

#[derive(Debug)]
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
