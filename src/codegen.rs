use std::path::Path;

use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
};
use inkwell::{AddressSpace, OptimizationLevel};

mod add_strict;
use add_strict::build_array_add_strict;

mod heap_array;
use heap_array::HeapArrayDescriptorTypes;

pub(crate) fn codegen() {
    const RANK: u32 = 2;

    let context = Context::create();
    let module = context.create_module("test_mod");
    let builder = context.create_builder();

    let size_type = context.i64_type();
    let elem_type = context.i32_type(); // ArrayU32 element type
    let ptr_type = context.ptr_type(AddressSpace::default());

    let mut type_cache = HeapArrayDescriptorTypes::new();
    let descriptor_type = type_cache.get(&context, ptr_type, size_type, RANK);

    let function = build_array_add_strict(
        &context,
        &module,
        &builder,
        descriptor_type,
        ptr_type,
        size_type,
        elem_type,
        RANK,
    );
    assert!(function.verify(true));

    Target::initialize_all(&InitializationConfig::default());
    let triple = TargetTriple::create("aarch64-apple-darwin26.0.0");
    let target = Target::from_triple(&triple).unwrap();
    let target_machine = target
        .create_target_machine(
            &triple,
            "apple-m2",
            "",
            OptimizationLevel::None,
            RelocMode::Default,
            CodeModel::Default,
        )
        .unwrap();
    println!("{}", module.to_string());
    module.set_triple(&triple);
    module.set_data_layout(&target_machine.get_target_data().get_data_layout());
    target_machine
        .write_to_file(&module, FileType::Object, Path::new("array_add.o"))
        .unwrap();
}
