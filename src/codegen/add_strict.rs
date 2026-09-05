use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{IntType, PointerType, StructType};
use inkwell::values::{FunctionValue, IntValue, PointerValue};
use inkwell::IntPredicate;

use super::heap_array::{
    build_element_count, build_load_buf, build_load_dim, build_new_heap_array,
};

/// Emits a loop `for i in 0..count { result[i] =  a[a_i] + b[b_i] }`, which should
/// let us reuse this for pervasive/broadcasting array add. Perhaps this could have
/// been planned out better; I ended up factoring out this part of the code because
/// the add functios were getting too large. This function takes a ton of arguments
/// but I think it's at a clean separation point, because it should be reusable for
/// pervasives. We might want to make it work across more instructions since we can
/// also apply this to subtraction etc.
pub(super) fn build_add_loop<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    function: FunctionValue<'ctx>,
    elem_type: inkwell::types::IntType<'ctx>,
    size_type: IntType<'ctx>,
    count: IntValue<'ctx>,
    a_buf: PointerValue<'ctx>,
    a_broadcast: bool,
    b_buf: PointerValue<'ctx>,
    b_broadcast: bool,
    result_buf: PointerValue<'ctx>,
    done_bb: inkwell::basic_block::BasicBlock<'ctx>,
    suffix: &str,
) {
    let preheader = builder.get_insert_block().unwrap();
    let loop_bb = context.append_basic_block(function, &format!("loop_{suffix}"));
    builder.build_unconditional_branch(loop_bb).unwrap();

    builder.position_at_end(loop_bb);
    let phi = builder
        .build_phi(size_type, &format!("i_{suffix}"))
        .unwrap();
    phi.add_incoming(&[(&size_type.const_int(0, false), preheader)]);
    let i = phi.as_basic_value().into_int_value();

    let zero = size_type.const_int(0, false);
    let a_idx = if a_broadcast { zero } else { i };
    let b_idx = if b_broadcast { zero } else { i };
    // Handle broadcasting, even though currently this doesn't see use.
    // I'll test if this logic works once I write `build_add`.
    // TODO: Test this

    let a_elem_ptr = unsafe {
        builder
            .build_gep(elem_type, a_buf, &[a_idx], "a_elem_ptr")
            .unwrap()
    };
    let a_val = builder
        .build_load(elem_type, a_elem_ptr, "a_val")
        .unwrap()
        .into_int_value();

    let b_elem_ptr = unsafe {
        builder
            .build_gep(elem_type, b_buf, &[b_idx], "b_elem_ptr")
            .unwrap()
    };
    let b_val = builder
        .build_load(elem_type, b_elem_ptr, "b_val")
        .unwrap()
        .into_int_value();

    let sum = builder.build_int_add(a_val, b_val, "sum").unwrap();

    let result_elem_ptr = unsafe {
        builder
            .build_gep(elem_type, result_buf, &[i], "result_elem_ptr")
            .unwrap()
    };
    builder.build_store(result_elem_ptr, sum).unwrap();

    let one = size_type.const_int(1, false);
    let next_i = builder.build_int_add(i, one, "next_i").unwrap();
    phi.add_incoming(&[(&next_i, loop_bb)]);

    let cont = builder
        .build_int_compare(IntPredicate::ULT, next_i, count, "cont")
        .unwrap();
    let after_bb = context.append_basic_block(function, &format!("after_{suffix}"));
    builder
        .build_conditional_branch(cont, loop_bb, after_bb)
        .unwrap();

    builder.position_at_end(after_bb);
    builder.build_unconditional_branch(done_bb).unwrap();
}

/// Generates `HeapArrayDescriptor* array_add_strict(HeapArrayDescriptor* a, HeapArrayDescriptor* b)`
/// Strict elementwise addition (no pervasion or broadcasting)
// TODO: Implement pervasion and broadcasting in `build_array_add`
pub fn build_array_add_strict<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    descriptor_type: StructType<'ctx>,
    ptr_type: PointerType<'ctx>,
    size_type: IntType<'ctx>,
    elem_type: inkwell::types::IntType<'ctx>,
    rank: u32,
) -> FunctionValue<'ctx> {
    let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    let function = module.add_function("array_add_strict", fn_type, None);

    let entry = context.append_basic_block(function, "entry");
    let mismatch_bb = context.append_basic_block(function, "shape_mismatch");
    let ok_bb = context.append_basic_block(function, "ok");

    builder.position_at_end(entry); // Main
    let a_ptr = function.get_nth_param(0).unwrap().into_pointer_value();
    let b_ptr = function.get_nth_param(1).unwrap().into_pointer_value();

    let a_dims = builder
        .build_struct_gep(descriptor_type, a_ptr, 3, "a_dims")
        .unwrap();
    let b_dims = builder
        .build_struct_gep(descriptor_type, b_ptr, 3, "b_dims")
        .unwrap();

    // Shapes must match exactly
    let mut shapes_equal = context.bool_type().const_int(1, false);
    for i in 0..rank {
        let v_a = build_load_dim(builder, size_type, rank, a_dims, i, "a_shape");
        let v_b = build_load_dim(builder, size_type, rank, b_dims, i, "b_shape");
        let eq = builder
            .build_int_compare(IntPredicate::EQ, v_a, v_b, "dim_eq")
            .unwrap();
        shapes_equal = builder.build_and(shapes_equal, eq, "shapes_equal").unwrap();
    }
    builder
        .build_conditional_branch(shapes_equal, ok_bb, mismatch_bb)
        .unwrap();

    builder.position_at_end(mismatch_bb); // Mismatched shapes
    let abort_fn = module.get_function("abort").unwrap_or_else(|| {
        module.add_function("abort", context.void_type().fn_type(&[], false), None)
    }); // Include libc abort() call
    builder.build_call(abort_fn, &[], "abort_call").unwrap();
    builder.build_unreachable().unwrap();
    // TODO: handle errors

    builder.position_at_end(ok_bb); // Single elementwise loop
    let count = build_element_count(builder, size_type, rank, a_dims, "count");
    let result = build_new_heap_array(
        builder,
        descriptor_type,
        size_type,
        elem_type,
        rank,
        count,
        a_dims,
    );
    let a_buf = build_load_buf(builder, descriptor_type, ptr_type, a_ptr, "a");
    let b_buf = build_load_buf(builder, descriptor_type, ptr_type, b_ptr, "b");
    let result_buf = build_load_buf(builder, descriptor_type, ptr_type, result, "result");

    let done_bb = context.append_basic_block(function, "done");
    build_add_loop(
        context, builder, function, elem_type, size_type, count, a_buf, false, b_buf, false,
        result_buf, done_bb, "ew",
    );

    builder.position_at_end(done_bb); // End block for the add loop to jump to
    builder.build_return(Some(&result)).unwrap();

    function
}
