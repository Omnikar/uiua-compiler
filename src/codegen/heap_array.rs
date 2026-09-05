use std::collections::HashMap;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::types::{IntType, PointerType, StructType};
use inkwell::values::{IntValue, PointerValue};

/// Container for the various HAD types (we'll need one per array rank)
pub(super) struct HeapArrayDescriptorTypes<'ctx> {
    descriptors: HashMap<u32, StructType<'ctx>>,
}

impl<'ctx> HeapArrayDescriptorTypes<'ctx> {
    pub(super) fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
        }
    }

    pub(super) fn get(
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
pub(super) fn build_load_dim<'ctx>(
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
pub(super) fn build_store_dim<'ctx>(
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
pub(super) fn build_load_buf<'ctx>(
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
pub(super) fn build_element_count<'ctx>(
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

pub(super) fn build_new_heap_array<'ctx>(
    builder: &Builder<'ctx>,
    descriptor_type: StructType<'ctx>,
    size_type: IntType<'ctx>,
    elem_type: inkwell::types::IntType<'ctx>,
    rank: u32,
    count: IntValue<'ctx>,
    // Should this function calculate `count` from the other given params using build_element_count()?
    src_dims_ptr: PointerValue<'ctx>,
) -> PointerValue<'ctx> {
    // I've decided to heap-allocate the descriptor as well as the data buffer
    // TODO: Add support for a reference count field
    let result = builder.build_malloc(descriptor_type, "result").unwrap();

    // buf = malloc(count * sizeof(elem))
    let buf = builder
        .build_array_malloc(elem_type, count, "result_buf_alloc")
        .unwrap();

    // store buf ptr
    let buf_field = builder
        .build_struct_gep(descriptor_type, result, 0, "result_buf_field")
        .unwrap();
    builder.build_store(buf_field, buf).unwrap();

    // offset = 0
    let offset_field = builder
        .build_struct_gep(descriptor_type, result, 1, "result_offset_field")
        .unwrap();
    builder
        .build_store(offset_field, size_type.const_int(0, false))
        .unwrap();

    // copy src_dims into result.dims
    let dims_field = builder
        .build_struct_gep(descriptor_type, result, 3, "result_dims_field")
        .unwrap();
    let mut dim_values = Vec::with_capacity(rank as usize);
    for i in 0..rank {
        let v = build_load_dim(builder, size_type, rank, src_dims_ptr, i, "src_copy");
        build_store_dim(builder, size_type, rank, dims_field, i, v, "result_dims");
        dim_values.push(v);
    }

    let strides_field = builder
        .build_struct_gep(descriptor_type, result, 2, "result_strides_field")
        .unwrap();
    let mut running = size_type.const_int(1, false);
    // Last axis has stride 1; walk backwards accumulating products of trailing dims
    // Stride for dim i is the product of all trailing dims, hence the .rev()
    let mut strides = vec![size_type.const_int(1, false); rank as usize];
    for i in (0..rank).rev() {
        strides[i as usize] = running;
        if i != 0 {
            running = builder
                .build_int_mul(running, dim_values[i as usize], "stride_acc")
                .unwrap();
        }
    }
    for i in 0..rank {
        build_store_dim(
            builder,
            size_type,
            rank,
            strides_field,
            i,
            strides[i as usize],
            "result_strides",
        );
    }

    result
}
