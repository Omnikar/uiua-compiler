use bidimap::BiMap;
use itertools::Itertools;
use petgraph::visit::{EdgeRef, IntoEdgeReferences, IntoNodeReferences};
use serde::{Deserialize, Serialize, de::Error};
use std::collections::{HashMap, HashSet};

pub use petgraph::stable_graph::NodeIndex;
pub type Graph<N> = petgraph::stable_graph::StableDiGraph<N, (usize, usize)>;

pub trait FunctionNode {
    fn is_input(&self) -> bool;
    fn is_output(&self) -> bool;
    fn input() -> Self;
    fn output() -> Self;
}

type FunctionSimpleGraph<'a, Node, NodeMeta> =
    petgraph::Graph<(&'a Node, &'a NodeMeta, usize), HashSet<(usize, usize)>>;

#[derive(Debug, Clone)]
pub struct Function<Meta, Node, NodeMeta> {
    pub meta: Meta,
    pub graph: Graph<Node>,
    pub input_idx: NodeIndex,
    pub output_idx: NodeIndex,
    pub node_metas: HashMap<NodeIndex, NodeMeta>,
    pub spans: HashMap<NodeIndex, usize>,
}
impl<Meta, Node, NodeMeta> Function<Meta, Node, NodeMeta> {
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

    /// Turn the IR graph, which is usually a multigraph, into a simple graph by combining edge weights into `HashSet`s
    ///
    /// Returns the new graph along with a bijective map from the old node indices to the new node indices.
    /// Returns an unstable graph because `petgraph` isomorphism algorithms require their inputs to implement `NodeCompactIndexable`, which stable graphs do not because it is possible for node deletions to leave unused indices.
    pub fn simple_graph(
        &self,
    ) -> (
        FunctionSimpleGraph<'_, Node, NodeMeta>,
        BiMap<NodeIndex, NodeIndex>,
    ) {
        let mut new_graph = petgraph::Graph::new();
        let mut node_idx_map = BiMap::new();
        for (node_idx, node) in self.graph.node_references() {
            let new_idx = new_graph.add_node((
                node,
                &self.node_metas[&node_idx],
                self.spans.get(&node_idx).copied().unwrap_or_default(),
            ));
            node_idx_map.insert(node_idx, new_idx);
        }

        for e in self.graph.edge_references() {
            let new_source = *node_idx_map.get_by_left(&e.source()).unwrap();
            let new_target = *node_idx_map.get_by_left(&e.target()).unwrap();
            let e_idx = if let Some(e) = new_graph.edges_connecting(new_source, new_target).next() {
                e.id()
            } else {
                new_graph.add_edge(new_source, new_target, HashSet::new())
            };
            new_graph[e_idx].insert(*e.weight());
        }

        (new_graph, node_idx_map)
    }
}

impl<Meta, Node, NodeMeta> PartialEq for Function<Meta, Node, NodeMeta>
where
    Meta: PartialEq,
    Node: PartialEq + Clone,
    NodeMeta: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.meta == other.meta && {
            let (simple_a, map_a) = self.simple_graph();
            let (simple_b, map_b) = other.simple_graph();
            map_a.get_by_left(&self.input_idx) == map_b.get_by_left(&other.input_idx)
                && map_a.get_by_left(&self.output_idx) == map_b.get_by_left(&other.output_idx)
                && {
                    petgraph::algo::is_isomorphic_matching(
                        &simple_a,
                        &simple_b,
                        |a, b| a == b,
                        |a, b| a == b,
                    )
                }
        }
    }
}
impl<Meta, Node, NodeMeta> Eq for Function<Meta, Node, NodeMeta>
where
    Meta: Eq,
    Node: Eq + Clone,
    NodeMeta: Eq,
{
}

type FunctionSerializable<Meta, Node, NodeMeta> =
    (Meta, Vec<(Vec<usize>, Node, Vec<usize>, NodeMeta, usize)>);
impl<Meta, Node, NodeMeta> From<&Function<Meta, Node, NodeMeta>>
    for FunctionSerializable<Meta, Node, NodeMeta>
where
    Meta: Clone,
    Node: Clone,
    NodeMeta: Clone + Default,
{
    fn from(func: &Function<Meta, Node, NodeMeta>) -> Self {
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
                func.node_metas.get(&node_idx).cloned().unwrap_or_default(),
                func.spans.get(&node_idx).copied().unwrap_or(0),
            ));
        }

        (func.meta.clone(), vec)
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
impl<Meta, Node, NodeMeta> TryFrom<FunctionSerializable<Meta, Node, NodeMeta>>
    for Function<Meta, Node, NodeMeta>
where
    Node: FunctionNode,
{
    type Error = FunctionDeserializeError;
    fn try_from(data: FunctionSerializable<Meta, Node, NodeMeta>) -> Result<Self, Self::Error> {
        let mut graph = Graph::<Node>::new();
        let mut node_metas = HashMap::<NodeIndex, NodeMeta>::new();
        let mut spans = HashMap::<NodeIndex, usize>::new();

        let mut idents = HashMap::<usize, (NodeIndex, usize)>::new();

        let mut instrs = data.1.into_iter();
        let (input_idents, input_meta, input_span) = instrs
            .next()
            .and_then(|(idents, node, _, meta, span)| {
                node.is_input().then_some((idents, meta, span))
            })
            .ok_or(FunctionDeserializeError::MissingInput)?;

        let input_idx = graph.add_node(Node::input());
        for (out_i, ident) in input_idents.into_iter().enumerate() {
            idents.insert(ident, (input_idx, out_i));
        }
        node_metas.insert(input_idx, input_meta);
        spans.insert(input_idx, input_span);

        for (out_idents, node, in_idents, node_meta, span) in instrs {
            let is_output = node.is_output();

            let new_node_idx = graph.add_node(node);
            node_metas.insert(new_node_idx, node_meta);
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
                    meta: data.0,
                    graph,
                    input_idx,
                    output_idx: new_node_idx,
                    node_metas,
                    spans,
                });
            }
        }

        Err(FunctionDeserializeError::MissingOutupt)
    }
}

impl<Meta, Node, NodeMeta> Serialize for Function<Meta, Node, NodeMeta>
where
    Meta: Serialize + Clone,
    Node: Serialize + Clone,
    NodeMeta: Serialize + Clone + Default,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        FunctionSerializable::from(self).serialize(serializer)
    }
}
impl<'de, Meta, Node, NodeMeta> Deserialize<'de> for Function<Meta, Node, NodeMeta>
where
    Meta: Deserialize<'de>,
    Node: Deserialize<'de> + FunctionNode,
    NodeMeta: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        FunctionSerializable::deserialize(deserializer)
            .and_then(|x| Function::try_from(x).map_err(D::Error::custom))
    }
}

/// Post-processes RON to convert
/// ```
/// [
///     1,
///     2,
///     3,
/// ]
/// ```
/// into
/// ```
/// [1, 2, 3]
/// ```
/// while leaving composites of other structures alone.
pub fn flatten_ron_number_lists(ron: &str) -> String {
    regex::Regex::new(r"\[\s*(?:\d+\s*,\s*)*\d*\s*\]")
        .unwrap()
        .replace_all(ron, |caps: &regex::Captures| {
            caps[0]
                .chars()
                .flat_map(|c| [(!c.is_whitespace()).then_some(c), (c == ',').then_some(' ')])
                .flatten()
                .collect::<String>()
        })
        .into_owned()
        .replace(", ]", "]")
}
