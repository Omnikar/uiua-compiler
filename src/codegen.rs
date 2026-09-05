use std::path::Path;

use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
};
use inkwell::{AddressSpace, OptimizationLevel};

mod heap_array;
mod add;

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

// TODO: Find out where the actual primitive implementations are gonna go
// TODO: Implement the primitives...

