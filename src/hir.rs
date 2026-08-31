use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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
        let ron = crate::generic_ir::flatten_ron_number_lists(&ron);
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ir_comparison() {
        let func1_s = "
            ((), [
                ([0, 1], Input, [], (), 0),
                ([2], FuncPrim(ADD), [0, 1], (), 0),
                ([3], FuncPrim(MUL), [0, 1], (), 0),
                ([4], FuncPrim(SUB), [2, 3], (), 0),
                ([], Output, [4], (), 0),
            ])
        ";
        let func2_s = "
            ((), [
                ([0, 1], Input, [], (), 0),
                ([2], FuncPrim(MUL), [0, 1], (), 0),
                ([3], FuncPrim(ADD), [0, 1], (), 0),
                ([4], FuncPrim(SUB), [3, 2], (), 0),
                ([], Output, [4], (), 0),
            ])
        ";

        let func1: Function = ron::from_str(func1_s).unwrap();
        let func2: Function = ron::from_str(func2_s).unwrap();

        assert_eq!(func1, func2);
    }
}
