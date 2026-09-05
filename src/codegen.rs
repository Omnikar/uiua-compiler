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

/// Container for the various HAD types (we'll need one per array rank)
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
        rank: u32, // Please never have an array with over 4 billion dimensions
    ) -> StructType<'ctx> {
        *self.descriptors.entry(rank).or_insert_with(|| {
            let name = format!("HeapArrayDescriptor{rank}");
            let ty = context.opaque_struct_type(&name); // Name the struct
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

/// Read from a HAD's dims array
fn build_load_dim<'ctx>(
    builder: &Builder<'ctx>,
    size_type: IntType<'ctx>,
    rank: u32,
    arr_ptr: PointerValue<'ctx>,
    i: u32,
    label: &str,
) -> IntValue<'ctx> {
    let array_type = size_type.array_type(rank);
    let zero = size_type.const_int(0, false);
    let idx = size_type.const_int(i as u64, false);
    // TODO: Figure out how we're gonna end up doing the sizing of these types

    let elem_ptr = unsafe {
        // "GEP is very likely to segfault if indexes are used incorrectly" but I'm pretty sure this works
        builder
            .build_gep(
                // get element pointer
                array_type,
                arr_ptr,
                &[zero, idx],
                &format!("{label}_gep{i}"),
            )
            .unwrap()
    };
    builder
        .build_load(size_type, elem_ptr, &format!("{label}_val{i}"))
        .unwrap()
        .into_int_value()
}

/// Write to a HAD's dims array
fn build_store_dim<'ctx>(
    builder: &Builder<'ctx>,
    size_type: IntType<'ctx>,
    rank: u32,
    arr_ptr: PointerValue<'ctx>,
    i: u32,
    value: IntValue<'ctx>,
    label: &str,
) {
    let array_type = size_type.array_type(rank);
    let zero = size_type.const_int(0, false);
    let idx = size_type.const_int(i as u64, false);
    let elem_ptr = unsafe {
        // "GEP is very likely to segfault if indexes are used incorrectly" but I'm pretty sure this works
        builder
            .build_gep(
                array_type,
                arr_ptr,
                &[zero, idx],
                &format!("{label}_gep{i}"),
            )
            .unwrap()
    };
    builder.build_store(elem_ptr, value).unwrap();
}

/// Load a HAD's buffer address
fn build_load_buf<'ctx>(
    builder: &Builder<'ctx>,
    descriptor_type: StructType<'ctx>,
    ptr_type: PointerType<'ctx>,
    desc_ptr: PointerValue<'ctx>,
    label: &str,
) -> PointerValue<'ctx> {
    let field = builder
        .build_struct_gep(descriptor_type, desc_ptr, 0, &format!("{label}_buf_field"))
        .unwrap();
    builder
        .build_load(ptr_type, field, &format!("{label}_buf"))
        .unwrap()
        .into_pointer_value()
}

/// Compute array length = product of dims
fn build_element_count<'ctx>(
    builder: &Builder<'ctx>,
    size_type: IntType<'ctx>,
    rank: u32,
    dims_ptr: PointerValue<'ctx>,
    label: &str,
) -> IntValue<'ctx> {
    let mut accumulator = size_type.const_int(1, false);
    for i in 0..rank {
        accumulator = builder
            .build_int_mul(
                accumulator,
                build_load_dim(builder, size_type, rank, dims_ptr, i, label),
                &format!("{label}_acc{i}"),
            )
            .unwrap();
    }
    accumulator
}

// TODO: make an allocator for new HADs
// TODO: find out where the actual primitive implementations are gonna go
// TODO: implement the primitives...
// TODO: Make an allocator for new HADs
// TODO: Find out where the actual primitive implementations are gonna go
// TODO: Implement the primitives...

