use std::collections::HashMap;

use crate::hir::{Binding, Function, Graph, Hir, Node, NodeIndex};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    // SigCheckError(uiua::SigCheckError),
    #[error("{0}")]
    UiuaError(#[from] uiua::UiuaError),
    #[error("error: {0}")]
    Other(String),
}

/// (Node index, output index)
type Stack = Vec<(NodeIndex, usize)>;
struct WorkingFuncGraph {
    graph: Graph<Node, (usize, usize)>,
    input_idx: NodeIndex,
    output_idx: NodeIndex,
    stack: Stack,
    under_stack: Stack,
    arg_count: usize,
    spans: HashMap<NodeIndex, usize>,
}

impl WorkingFuncGraph {
    fn empty() -> Self {
        let mut graph = Graph::new();
        let input_idx = graph.add_node(Node::Input);
        let output_idx = graph.add_node(Node::Output);
        Self {
            graph,
            input_idx,
            output_idx,
            stack: Stack::new(),
            under_stack: Stack::new(),
            arg_count: 0,
            spans: HashMap::new(),
        }
    }
    /// Add arguments to the function to meet a minimum number on the stack
    fn extend_args(&mut self, min_args: usize) {
        for _ in 0..min_args.saturating_sub(self.stack.len()) {
            self.stack.insert(0, (self.input_idx, self.arg_count));
            self.arg_count += 1;
        }
    }

    // If any of the following `unwrap`s or indexing operations panic, it is a compiler bug.

    /// Pop and return the top stack value
    fn stack_pop(&mut self) -> (NodeIndex, usize) {
        self.stack.pop().unwrap()
    }
    /// Read the top stack value without popping it
    fn stack_top(&self) -> (NodeIndex, usize) {
        *self.stack.last().unwrap()
    }
    /// Read the nth stack value from the top, where 0 indicates the top stack value
    fn stack_n(&self, n: usize) -> (NodeIndex, usize) {
        self.stack[self.stack.len() - n - 1]
    }
}

pub fn construct_hir(uasm: &uiua::Assembly) -> Result<Hir, Error> {
    let mut ir = Hir {
        spans: uasm.spans.iter().cloned().collect(),
        datadefs: Vec::new(),
        bindings: Vec::new(),
    };

    for binding_info in &uasm.bindings {
        use uiua::BindingKind as Bk;
        match &binding_info.kind {
            Bk::Const(value) => todo!(),
            Bk::Func(function) => {
                let uiua_node = &uasm[function];
                let binding = Binding {
                    span: binding_info.span.clone(),
                    func_id: function.id.clone(),
                    hash: 0, // FIXME
                    func: simulate_data_flow(uiua_node)?,
                };
                ir.bindings.push(binding);
            }
            Bk::Module(module) => todo!(),
            _ => continue,
        }
    }

    Ok(ir)
}

fn simulate_data_flow(uiua_node: &uiua::Node) -> Result<Function, Error> {
    let mut func_graph = WorkingFuncGraph::empty();
    process_node(uiua_node, &mut func_graph)?;
    for (in_i, (node_idx, out_i)) in func_graph.stack.into_iter().enumerate() {
        func_graph
            .graph
            .add_edge(func_graph.output_idx, node_idx, (out_i, in_i));
    }
    Ok(Function {
        graph: func_graph.graph,
        input_idx: func_graph.input_idx,
        output_idx: func_graph.output_idx,
        spans: func_graph.spans,
    })
}

fn process_node(uiua_node: &uiua::Node, func_graph: &mut WorkingFuncGraph) -> Result<(), Error> {
    let sig = uiua_node.sig().map_err(|e| Error::Other(e.to_string()))?;
    func_graph.extend_args(sig.args());

    /// Used to error if a modifier was passed any amount of functions other than one
    fn one_func(prim: uiua::Primitive, funcs: &[uiua::SigNode]) -> Result<&uiua::SigNode, Error> {
        if funcs.len() != 1 {
            // bail!(
            //     "{} passed {} functions instead of 1",
            //     prim.format(),
            //     funcs.len()
            // );
            return Err(Error::Other("TODO".into()));
        }
        Ok(&funcs[0])
    }

    use uiua::ImplPrimitive as Ip;
    use uiua::Node as UNode;
    use uiua::Primitive as Pr;
    match uiua_node {
        UNode::CustomInverse(custom_inverse, span) => {
            let uiua_node = custom_inverse
                .normal
                .as_ref()
                .map_err(|e| Error::Other(e.to_string()))?;
            process_node(&uiua_node.node, func_graph)?;
        }
        UNode::PushUnder(n, _span) => func_graph
            .under_stack
            .extend(func_graph.stack.drain(func_graph.stack.len() - n..).rev()),
        UNode::CopyToUnder(n, _span) => func_graph
            .under_stack
            .extend(func_graph.stack[func_graph.stack.len() - n..].iter().rev()),
        UNode::PopUnder(n, _span) => func_graph.stack.extend(
            func_graph
                .under_stack
                .drain(func_graph.under_stack.len() - n..),
        ),
        UNode::Prim(Pr::Identity, _span) => {}
        UNode::Prim(Pr::Pop, _span) => {
            func_graph.stack_pop();
        }
        UNode::Prim(Pr::Flip, _span) => {
            let i = func_graph.stack.len() - 2;
            func_graph.stack[i..].reverse();
        }
        UNode::Mod(Pr::On, funcs, _span) => {
            let func = one_func(Pr::On, funcs)?;
            let preserved = func_graph.stack_top();
            process_node(&func.node, func_graph)?;
            func_graph.stack.push(preserved);
        }
        _ if let Some((node, span)) = {
            match uiua_node {
                UNode::Prim(prim, span) => Some((Node::PrimFunc(*prim), span)),
                UNode::Mod(prim, funcs, span) => Some((
                    Node::PrimMod(
                        *prim,
                        funcs
                            .iter()
                            .map(|sig_node| simulate_data_flow(&sig_node.node))
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                    span,
                )),
                _ => None,
            }
        } =>
        {
            let new_node_idx = func_graph.graph.add_node(node);
            func_graph.spans.insert(new_node_idx, *span);
            for (in_i, (arg, out_i)) in func_graph
                .stack
                .drain(func_graph.stack.len() - sig.args()..)
                .rev()
                .enumerate()
            {
                // Each edge is given a weight consisting of a tuple of two numbers. The first number indicates which output from the depended-upon node is being used, and the second number indicates which input for the dependent node it is used for.
                // So a `Sub` node will have two arrows pointing out of it, one arrow will have weight (_, 0), corresponding to the left argument, and the other arrow will have weight (_, 1), corresponding to the right argument.
                // As another example, consider an `UnKeep` node. An arrow pointing toward it with weight (0, _) indicates that something is using the run-length output, whereas an arrow pointing toward it with weight (1, _) indicates that something is using the adjacent-deduplicated output.
                func_graph.graph.add_edge(new_node_idx, arg, (out_i, in_i));
            }

            for out_i in (0..sig.outputs()).rev() {
                func_graph.stack.push((new_node_idx, out_i));
            }
        }
        _ => todo!(),
    }

    Ok(())
}
