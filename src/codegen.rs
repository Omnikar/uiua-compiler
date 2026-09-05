use std::collections::HashMap;
use std::path::Path;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::types::{IntType, PointerType, StructType};
use inkwell::values::{FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate, OptimizationLevel};

///
struct HeapArrayDescriptorTypes<'ctx> {
    descriptors: HashMap<u32, StructType<'ctx>>,
}

impl<'ctx> HeapArrayDescriptorTypes<'ctx> {
    fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
        }
    }

    fn get(
        &mut self,
        context: &'ctx Context,
        ptr_type: PointerType<'ctx>,
        size_type: IntType<'ctx>,
        rank: u32, // please never have an array with over 4 billion dimensions
    ) -> StructType<'ctx> {
        *self.descriptors.entry(rank).or_insert_with(|| {
            let name = format!("HeapArrayDescriptor{rank}");
            let ty = context.opaque_struct_type(&name);
            ty.set_body(
                &[
                    ptr_type.into(),                   // buf
                    size_type.into(),                  // offset
                    size_type.array_type(rank).into(), // strides
                    size_type.array_type(rank).into(), // dims
                ],
                false,
            );
            ty
        })
    }
}
