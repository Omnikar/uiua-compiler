use std::path::Path;

use clap::{Args, ValueEnum};
use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::{AddressSpace, OptimizationLevel};

mod add_strict;
use add_strict::build_array_add_strict;

mod heap_array;
use heap_array::HeapArrayDescriptorTypes;

#[derive(Args, Debug, Clone)]
pub struct TargetArgs {
    /// LLVM target triple
    #[arg(long)]
    pub target: Option<String>,

    /// Target CPU (eg 'apple-m2')
    #[arg(long)]
    pub cpu: Option<String>,

    /// Target features (eg '+thumb-mode,+vfp4' for embedded)
    #[arg(long, default_value = "")]
    pub features: String,

    /// Optimization level
    #[arg(short = 'O', long = "opt", value_enum, default_value_t = OptLevel::None)]
    pub opt_level: OptLevel,

    /// Relocation model
    #[arg(long, value_enum, default_value_t = Reloc::Default)]
    pub reloc_mode: Reloc,

    /// Code model
    #[arg(long, value_enum, default_value_t = CodeModelArg::Default)]
    pub code_model: CodeModelArg,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptLevel {
    None,
    Less,
    Default,
    Aggressive,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reloc {
    Default,
    Static,
    Pic,
    DynamicNoPic,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeModelArg {
    Default,
    Small,
    Kernel,
    Medium,
    Large,
}

impl TargetArgs {
    pub fn create_target_machine(&self) -> Result<(TargetMachine, TargetTriple), String> {
        Target::initialize_all(&InitializationConfig::default());

        let triple = if let Some(t) = &self.target {
            TargetTriple::create(t)
        } else {
            TargetMachine::get_default_triple()
        };
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;

        let cpu = self.cpu.as_deref().unwrap_or("generic");

        let opt = match self.opt_level {
            OptLevel::None => OptimizationLevel::None,
            OptLevel::Less => OptimizationLevel::Less,
            OptLevel::Default => OptimizationLevel::Default,
            OptLevel::Aggressive => OptimizationLevel::Aggressive,
        };

        let reloc = match self.reloc_mode {
            Reloc::Default => RelocMode::Default,
            Reloc::Static => RelocMode::Static,
            Reloc::Pic => RelocMode::PIC,
            Reloc::DynamicNoPic => RelocMode::DynamicNoPic,
        };

        let model = match self.code_model {
            CodeModelArg::Default => CodeModel::Default,
            CodeModelArg::Small => CodeModel::Small,
            CodeModelArg::Kernel => CodeModel::Kernel,
            CodeModelArg::Medium => CodeModel::Medium,
            CodeModelArg::Large => CodeModel::Large,
        };

        let machine = target
            .create_target_machine(&triple, cpu, &self.features, opt, reloc, model)
            .ok_or_else(|| "Failed to create LLVM TargetMachine".to_string())?;

        Ok((machine, triple))
    }
}

pub(crate) fn codegen(
    target_args: &TargetArgs,
    emit_object: bool,
    output_path: Option<&Path>,
) -> Result<(), String> {
    let (target_machine, triple) = target_args.create_target_machine()?;
    let target_data = target_machine.get_target_data();

    let context = Context::create();
    let module = context.create_module("uiua_program");
    let builder = context.create_builder();

    let size_type = context.ptr_sized_int_type(&target_data, None);
    let elem_type = context.i32_type(); // ArrayU32 element type
    let ptr_type = context.ptr_type(AddressSpace::default());

    let mut types = HeapArrayDescriptorTypes::new();
    for rank in 1..=2 {
        let descriptor_type = types.get(&context, ptr_type, size_type, rank);

        let function = build_array_add_strict(
            &context,
            &module,
            &builder,
            descriptor_type,
            ptr_type,
            size_type,
            elem_type,
            rank,
        );
        assert!(function.verify(true));
    }

    module.set_triple(&triple);
    module.set_data_layout(&target_data.get_data_layout());

    if emit_object {
        let out_file = output_path.unwrap_or_else(|| Path::new("a.o"));
        target_machine
            .write_to_file(&module, FileType::Object, out_file)
            .map_err(|e| e.to_string())?;
    } else if let Some(path) = output_path {
        module.print_to_file(path).map_err(|e| e.to_string())?;
    } else {
        print!("{}", module.to_string());
    }

    Ok(())
}
