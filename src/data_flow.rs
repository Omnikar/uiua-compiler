use itertools::Itertools;
use std::collections::HashMap;

use crate::generic_ir::{Graph, NodeIndex};
use crate::hir::{Binding, Function, Hir, Node, Struct};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    UiuaError(#[from] uiua::UiuaError),
    #[error("{0}")]
    Other(String),
}

/// (node index, output index)
type Stack = Vec<(NodeIndex, usize)>;
struct WorkingFuncGraph {
    graph: Graph<Node>,
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
    let mut hir = Hir {
        structs: Vec::new(),
        enums: Vec::new(),
        bindings: Vec::new(),
        main: None,
        spans: uasm.spans.iter().cloned().collect(),
        files: uasm
            .inputs
            .files
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().to_string()))
            .collect(),
    };

    for binding_info in &uasm.bindings {
        use uiua::BindingKind as Bk;
        match &binding_info.kind {
            Bk::Func(function) => {
                let uiua_node = &uasm[function];
                let binding = Binding {
                    span: binding_info.span.clone(),
                    func_id: function.id.clone(),
                    hash: function.hash(),
                    func: simulate_data_flow(uiua_node)?,
                };
                hir.bindings.push(binding);
            }
            Bk::Const(_value) => {
                // Constants are currently not compiled into the IR
            }
            _ => {}
        }
    }
    use uiua::BindingKind as Bk;
    use uiua::LocalIndex;
    for (exp_name, exp_index) in &*uasm.exports {
        if let Bk::Module(module) = &uasm.bindings[*exp_index].kind
            && let Some(LocalIndex {
                index: type_const_index,
                ..
            }) = module
                .names
                .get_only("t", uiua::LookupPreference::Function, uasm)
            && let Some(LocalIndex {
                index: fields_const_index,
                ..
            }) = module
                .names
                .get_only("t", uiua::LookupPreference::Function, uasm)
            && let Bk::Const(Some(uiua::Value::Box(type_array))) =
                &uasm.bindings[type_const_index].kind
            && let Bk::Const(Some(uiua::Value::Box(fields_array))) =
                &uasm.bindings[fields_const_index].kind
        {
            let mut struct_def = Struct {
                name: exp_name.into(),
                fields: Vec::new(),
            };
            for (elem_name, elem_type) in fields_array.data().iter().zip(type_array.data()) {
                if let uiua::Value::Char(name_arr) = elem_name.as_ref() {
                    struct_def
                        .fields
                        .push((name_arr.elements().collect(), elem_type.as_ref().clone()));
                }
            }
        }
    }

    if !uasm.root.is_empty() {
        let func = simulate_data_flow(&uasm.root)?;
        hir.main = Some((func, uasm.root.span().unwrap_or(0)));
    }

    Ok(hir)
}

fn simulate_data_flow(uiua_node: &uiua::Node) -> Result<Function, Error> {
    let mut func_graph = WorkingFuncGraph::empty();
    process_node(uiua_node, &mut func_graph)?;
    for (in_i, (node_idx, out_i)) in func_graph.stack.into_iter().rev().enumerate() {
        func_graph
            .graph
            .add_edge(func_graph.output_idx, node_idx, (out_i, in_i));
    }
    let node_metas = func_graph.graph.node_indices().map(|k| (k, ())).collect();
    Ok(Function {
        meta: (),
        graph: func_graph.graph,
        input_idx: func_graph.input_idx,
        output_idx: func_graph.output_idx,
        node_metas,
        spans: func_graph.spans,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "This function comprises one giant `match` block for handling stack manipulation"
)]
fn process_node(uiua_node: &uiua::Node, func_graph: &mut WorkingFuncGraph) -> Result<(), Error> {
    let sig = uiua_node.sig().map_err(|e| Error::Other(e.to_string()))?;
    func_graph.extend_args(sig.args());

    use uiua::ImplPrimitive as Ip;
    use uiua::Node as UNode;
    use uiua::Primitive as Pr;
    match uiua_node {
        UNode::CustomInverse(custom_inverse, _span) => {
            let uiua_node = custom_inverse
                .normal
                .as_ref()
                .map_err(|e| Error::Other(e.to_string()))?;
            process_node(&uiua_node.node, func_graph)?;
        }
        UNode::PushUnder(n, _span) => func_graph
            .under_stack
            .extend(drain_top_n(&mut func_graph.stack, *n).rev()),
        UNode::CopyToUnder(n, _span) => func_graph
            .under_stack
            .extend(top_n(&func_graph.stack, *n).iter().rev()),
        UNode::PopUnder(n, _span) => func_graph
            .stack
            .extend(drain_top_n(&mut func_graph.under_stack, *n)),
        UNode::Prim(Pr::Identity, _span) => {}
        UNode::Prim(Pr::Pop, _span) => {
            func_graph.stack_pop();
        }
        UNode::Prim(Pr::Dup, _span) => {
            func_graph.stack.push(func_graph.stack_top());
        }
        UNode::Prim(Pr::Flip, _span) => {
            let i = func_graph.stack.len() - 2;
            func_graph.stack[i..].reverse();
        }
        UNode::Mod(Pr::On, funcs, _span) => {
            let func = &funcs[0];
            let preserved = func_graph.stack_top();
            process_node(&func.node, func_graph)?;
            func_graph.stack.push(preserved);
        }
        UNode::ImplMod(Ip::OnSub(n), funcs, _span) => {
            let func = &funcs[0];
            let mut preserved = top_n(&func_graph.stack, *n).to_vec();
            process_node(&func.node, func_graph)?;
            func_graph.stack.append(&mut preserved);
        }
        UNode::Mod(Pr::By, funcs, _span) => {
            let func = &funcs[0];
            let n_args = func.sig.args();
            let preserved = func_graph.stack_n(n_args);
            func_graph
                .stack
                .insert(func_graph.stack.len() - n_args, preserved);
            process_node(&func.node, func_graph)?;
        }
        UNode::ImplMod(Ip::BySub(n), funcs, _span) => {
            let func = &funcs[0];
            let n_args = func.sig.args();
            let start_i = func_graph.stack.len() - n_args;
            let preserved = func_graph.stack[start_i..start_i + n].to_vec();
            func_graph.stack.splice(start_i..start_i, preserved);
            process_node(&func.node, func_graph)?;
        }
        UNode::Mod(Pr::Off, funcs, _span) => {
            let func = &funcs[0];
            let n_args = func.sig.args();
            let preserved = func_graph.stack_top();
            func_graph
                .stack
                .insert(func_graph.stack.len() - n_args, preserved);
            process_node(&func.node, func_graph)?;
        }
        UNode::ImplMod(Ip::OffSub(n), funcs, _span) => {
            let func = &funcs[0];
            let n_args = func.sig.args();
            let start_i = func_graph.stack.len() - n_args;
            let preserved = top_n(&func_graph.stack, *n).to_vec();
            func_graph.stack.splice(start_i..start_i, preserved);
            process_node(&func.node, func_graph)?;
        }
        UNode::Mod(Pr::With, funcs, _span) => {
            let func = &funcs[0];
            let n_args = func.sig.args();
            let preserved = func_graph.stack_n(n_args);
            process_node(&func.node, func_graph)?;
            func_graph.stack.push(preserved);
        }
        UNode::ImplMod(Ip::WithSub(n), funcs, _span) => {
            let func = &funcs[0];
            let n_args = func.sig.args();
            let start_i = func_graph.stack.len() - n_args;
            let mut preserved = func_graph.stack[start_i..start_i + n].to_vec();
            process_node(&func.node, func_graph)?;
            func_graph.stack.append(&mut preserved);
        }
        UNode::Mod(Pr::Dip, funcs, _span) => {
            let func = &funcs[0];
            let skipped = func_graph.stack_pop();
            process_node(&func.node, func_graph)?;
            func_graph.stack.push(skipped);
        }
        UNode::ImplMod(Ip::DipN(n), funcs, _span) => {
            let func = &funcs[0];
            let mut skipped = drain_top_n(&mut func_graph.stack, *n).collect_vec();
            process_node(&func.node, func_graph)?;
            func_graph.stack.append(&mut skipped);
        }
        UNode::Mod(Pr::Fork, funcs, _span) => {
            let reused = drain_top_n(&mut func_graph.stack, sig.args()).collect_vec();
            for func in funcs.iter().rev() {
                func_graph
                    .stack
                    .extend_from_slice(top_n(&reused, func.sig.args()));
                process_node(&func.node, func_graph)?;
            }
        }
        UNode::Mod(Pr::Bracket, funcs, _span) => {
            let mut args = drain_top_n(&mut func_graph.stack, sig.args())
                .rev()
                .collect_vec();
            for func in funcs.iter().rev() {
                func_graph
                    .stack
                    .extend(drain_top_n(&mut args, func.sig.args()).rev());
                process_node(&func.node, func_graph)?;
            }
        }
        UNode::ImplMod(Ip::SidedBracket(_sided_subscript), _funcs, _span) => {
            todo!("Subscripted `bracket` has not been implemented yet.")
        }
        UNode::Mod(Pr::Below, funcs, _span) => {
            let func = &funcs[0];
            func_graph
                .stack
                .extend(top_n(&func_graph.stack, func.sig.args()).to_vec());
            process_node(&func.node, func_graph)?;
        }
        UNode::Mod(Pr::Both, funcs, _span) => {
            let func = &funcs[0];
            let saved = drain_top_n(&mut func_graph.stack, func.sig.args()).collect_vec();
            process_node(&func.node, func_graph)?;
            func_graph.stack.extend(saved);
            process_node(&func.node, func_graph)?;
        }
        UNode::ImplMod(Ip::BothImpl(_subscript), _funcs, _span) => {
            todo!("Subscripted `both` has not been implemented yet.")
        }
        UNode::Run(sub_nodes) => {
            for sub_node in sub_nodes {
                process_node(sub_node, func_graph)?;
            }
        }
        UNode::Push(value) => {
            let new_node_idx = func_graph.graph.add_node(Node::Constant(value.clone()));
            func_graph.stack.push((new_node_idx, 0));
        }
        // --- Unimplemented fillers ---
        UNode::TrackCaller(sig_node) => {
            process_node(&sig_node.node, func_graph)?;
        }
        UNode::Label(..) | UNode::RemoveLabel(..) => {}
        // ---
        _ if let Some((node, span)) = {
            match uiua_node {
                UNode::Prim(prim, span) => Some((Node::FuncPrim(*prim), span)),
                UNode::ImplPrim(impl_prim, span) => Some((Node::FuncImplPrim(*impl_prim), span)),
                UNode::Mod(prim, funcs, span) => Some((
                    Node::ModPrim(
                        *prim,
                        funcs
                            .iter()
                            .map(|sig_node| simulate_data_flow(&sig_node.node))
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                    span,
                )),
                UNode::ImplMod(impl_prim, funcs, span) => Some((
                    Node::ModImplPrim(
                        *impl_prim,
                        funcs
                            .iter()
                            .map(|sig_node| simulate_data_flow(&sig_node.node))
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                    span,
                )),
                UNode::Call(func, span) => Some((Node::Call(func.clone()), span)),
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
        _ => todo!("{:?}", uiua_node),
    }

    Ok(())
}

fn top_n<T>(slice: &[T], n: usize) -> &[T] {
    &slice[slice.len() - n..]
}

fn drain_top_n<T>(stack: &mut Vec<T>, n: usize) -> impl DoubleEndedIterator<Item = T> {
    stack.drain(stack.len() - n..)
}
