use serde::{Deserialize, Serialize};
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
                caps[0]
                    .chars()
                    .flat_map(|c| [(!c.is_whitespace()).then_some(c), (c == ',').then_some(' ')])
                    .flatten()
                    .collect::<String>()
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
impl crate::generic_ir::FunctionNode for Node {
    fn is_input(&self) -> bool {
        matches!(self, Self::Input)
    }
    fn is_output(&self) -> bool {
        matches!(self, Self::Output)
    }
    fn input() -> Self {
        Self::Input
    }
    fn output() -> Self {
        Self::Output
    }
}

pub type Function = crate::generic_ir::Function<(), Node, ()>;
