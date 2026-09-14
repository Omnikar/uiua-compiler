#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use itertools::{Either, Itertools};

use super::{
    AnalyzeContext, Error, ErrorKind, FunctionTranslation, MirValue, ValueInfo, mir, types,
};
use types::ScalarInfo as S;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}
impl Side {
    fn select<T>(self, lhs: T, rhs: T) -> T {
        match self {
            Self::Left => lhs,
            Self::Right => rhs,
        }
    }

    fn place_first<T>(self, lhs: T, rhs: T) -> (T, T) {
        match self {
            Self::Left => (lhs, rhs),
            Self::Right => (rhs, lhs),
        }
    }
}
enum ScalarTypeMatch {
    Identical,
    Matching(Side, types::ScalarInfo, types::ScalarInfo),
    Mismatched,
}
fn try_match_scalar_types(lhs: types::ScalarInfo, rhs: types::ScalarInfo) -> ScalarTypeMatch {
    use ScalarTypeMatch as Stm;
    use Side::{Left, Right};
    use types::ScalarInfo as S;
    match (lhs, rhs) {
        (S::Bool(_), S::Bool(_))
        | (S::Int(_), S::Int(_))
        | (S::Float(_), S::Float(_))
        | (S::Char(_), S::Char(_)) => Stm::Identical,
        (S::Bool(l), S::Int(_)) => Stm::Matching(Left, S::Bool(l), S::Int(l.map(i64::from))),
        (S::Bool(l), S::Float(_)) => Stm::Matching(Left, S::Bool(l), S::Float(l.map(f64::from))),
        (S::Int(_), S::Bool(r)) => Stm::Matching(Right, S::Bool(r), S::Int(r.map(i64::from))),
        (S::Int(l), S::Float(_)) => Stm::Matching(Left, S::Int(l), S::Float(l.map(|l| l as f64))),
        (S::Float(_), S::Bool(r)) => Stm::Matching(Right, S::Bool(r), S::Float(r.map(f64::from))),
        (S::Float(_), S::Int(r)) => Stm::Matching(Right, S::Int(r), S::Float(r.map(|r| r as f64))),
        _ => Stm::Mismatched,
    }
}

/// Attempt to insert casts to match scalar types
fn try_match_types<'a>(
    func_name: &'static str,
    (lhs, lhs_info): (&mut MirValue, &mut &'a ValueInfo),
    (rhs, rhs_info): (&mut MirValue, &mut &'a ValueInfo),
    tr: &'a FunctionTranslation,
    ctx: AnalyzeContext,
) -> Result<(), Error> {
    use ValueInfo as V;
    match (*lhs_info, *rhs_info) {
        (V::Scalar(l), V::Scalar(r)) => match try_match_scalar_types(*l, *r) {
            ScalarTypeMatch::Identical => return Ok(()),
            ScalarTypeMatch::Matching(side, from, to) => {
                let (val, val_info) = side.select((lhs, lhs_info), (rhs, rhs_info));
                [(*val, *val_info)] = tr.add_node(
                    mir::MirOp::CastNum { from, to }.into(),
                    [V::Scalar(to)],
                    [*val],
                );
            }
            ScalarTypeMatch::Mismatched => ctx.error(ErrorKind::IncompatibleTypes(
                func_name,
                l.type_name().into(),
                r.type_name().into(),
            ))?,
        },
        (V::Scalar(scalar), V::Array(array)) => {
            let Some(array_scalar_type) = array.scalar_type() else {
                ctx.error(ErrorKind::IncompatibleTypes(
                    func_name,
                    scalar.type_name().into(),
                    array.type_name(),
                ))?
            };
            match try_match_scalar_types(*scalar, array_scalar_type) {
                ScalarTypeMatch::Identical => return Ok(()),
                ScalarTypeMatch::Matching(side, from, to) => match side {
                    Side::Left => {
                        [(*lhs, *lhs_info)] = tr.add_node(
                            mir::MirOp::CastNum { from, to }.into(),
                            [V::Scalar(to)],
                            [*lhs],
                        );
                    }
                    Side::Right => {
                        let mut new_info = rhs_info.clone();
                        *new_info.scalar_type_mut().unwrap() = to;
                        [(*rhs, *rhs_info)] = tr.add_node(
                            mir::MirOp::CastNum { from, to }.into(),
                            [new_info],
                            [*rhs],
                        );
                    }
                },
                ScalarTypeMatch::Mismatched => ctx.error(ErrorKind::IncompatibleTypes(
                    func_name,
                    scalar.type_name().into(),
                    array.type_name(),
                ))?,
            }
        }
        (V::Array(_), V::Scalar(_)) => {
            let mut ctx = ctx;
            let mut new_input_spans = ctx.input_spans.to_vec();
            new_input_spans.reverse();
            ctx.input_spans = &new_input_spans;
            try_match_types(func_name, (rhs, rhs_info), (lhs, lhs_info), tr, ctx)?;
        }
        (V::Array(l), V::Array(r)) => {
            let Some((lhs_scalar_type, rhs_scalar_type)) = l.scalar_type().zip(r.scalar_type())
            else {
                ctx.error(ErrorKind::IncompatibleTypes(
                    func_name,
                    l.type_name(),
                    r.type_name(),
                ))?
            };
            match try_match_scalar_types(lhs_scalar_type, rhs_scalar_type) {
                ScalarTypeMatch::Identical => return Ok(()),
                ScalarTypeMatch::Matching(side, from, to) => {
                    let (val, val_info) = side.select((lhs, lhs_info), (rhs, rhs_info));
                    let mut new_val_info = val_info.clone();
                    *new_val_info.scalar_type_mut().unwrap() = to;
                    [(*val, *val_info)] = tr.add_node(
                        mir::MirOp::CastNum { from, to }.into(),
                        [new_val_info],
                        [*val],
                    );
                }
                ScalarTypeMatch::Mismatched => ctx.error(ErrorKind::IncompatibleTypes(
                    func_name,
                    l.type_name(),
                    r.type_name(),
                ))?,
            }
        }
        _ => todo!(),
    }
    Ok(())
}

impl ValueInfo {
    pub fn upcast_scalars(&mut self, scalar_type: types::ScalarInfo) {
        use ScalarTypeMatch as Stm;
        match self {
            ValueInfo::Scalar(scalar) => {
                if let Stm::Matching(Side::Left, _, to) =
                    try_match_scalar_types(*scalar, scalar_type)
                {
                    *scalar = to;
                }
            }
            ValueInfo::Array(array) => match &mut **array {
                types::ArrayInfo::Known {
                    element_type,
                    value,
                } => {
                    element_type.upcast_scalars(scalar_type);
                    value
                        .data
                        .iter_mut()
                        .for_each(|x| x.upcast_scalars(scalar_type));
                }
                types::ArrayInfo::Ranked { element_type, .. }
                | types::ArrayInfo::Unranked { element_type, .. } => {
                    element_type.upcast_scalars(scalar_type);
                }
            },
            ValueInfo::Map(map) => {
                map.value_type.upcast_scalars(scalar_type);
            }
            _ => panic!("Compiler attempted invalid scalar upcast"),
        }
    }
}

type ShapeCheckList = Vec<Either<mir::MirOp, (Side, mir::MirOp)>>;
fn try_match_shapes_rec(
    func_name: &'static str,
    lhs_info: &ValueInfo,
    rhs_info: &ValueInfo,
    ctx: AnalyzeContext,
) -> Result<ShapeCheckList, Error> {
    use ValueInfo as V;
    match (lhs_info, rhs_info) {
        (V::Scalar(_), _) | (_, V::Scalar(_)) => Ok(Vec::new()),
        (V::Array(l_array), V::Array(r_array)) => {
            let lhs_shape = l_array
                .sym_shape()
                .ok_or_else(|| ctx.make_error(ErrorKind::Unranked(func_name)))?;
            let rhs_shape = r_array
                .sym_shape()
                .ok_or_else(|| ctx.make_error(ErrorKind::Unranked(func_name)))?;

            let mut checks = Vec::new();

            for (ax_i, eob) in lhs_shape.iter().zip_longest(rhs_shape.iter()).enumerate() {
                let itertools::EitherOrBoth::Both(l_ax, r_ax) = eob else {
                    continue;
                };
                if let Some((l_const, r_const)) = l_ax.as_const().zip(r_ax.as_const()) {
                    if l_const != 1 && r_const != 1 && l_const != r_const {
                        ctx.error(ErrorKind::IncompatibleShapes(
                            l_array.type_name(),
                            r_array.type_name(),
                        ))?;
                    }
                } else if let Some(l_const) = l_ax.as_const() {
                    checks.push(Either::Right((
                        Side::Right,
                        mir::MirOp::CheckAxis {
                            depth: 0,
                            ax_i,
                            length: l_const.cast_unsigned(),
                        },
                    )));
                } else if let Some(r_const) = r_ax.as_const() {
                    checks.push(Either::Right((
                        Side::Left,
                        mir::MirOp::CheckAxis {
                            depth: 0,
                            ax_i,
                            length: r_const.cast_unsigned(),
                        },
                    )));
                } else if let Some(0) = (l_ax.clone() - r_ax.clone()).as_const() {
                    // These unknown axes are known equal, no check needed
                } else {
                    checks.push(Either::Left(mir::MirOp::CheckAxes {
                        lhs_depth: 0,
                        lhs_ax_i: ax_i,
                        rhs_depth: 0,
                        rhs_ax_i: ax_i,
                    }));
                }
            }

            for check in try_match_shapes_rec(
                func_name,
                l_array.element_type(),
                r_array.element_type(),
                ctx,
            )? {
                checks.push(match check {
                    Either::Left(mir::MirOp::CheckAxes {
                        lhs_depth,
                        lhs_ax_i,
                        rhs_depth,
                        rhs_ax_i,
                    }) => Either::Left(mir::MirOp::CheckAxes {
                        lhs_depth: lhs_depth + 1,
                        lhs_ax_i,
                        rhs_depth: rhs_depth + 1,
                        rhs_ax_i,
                    }),
                    Either::Right((
                        side,
                        mir::MirOp::CheckAxis {
                            depth,
                            ax_i,
                            length,
                        },
                    )) => Either::Right((
                        side,
                        mir::MirOp::CheckAxis {
                            depth: depth + 1,
                            ax_i,
                            length,
                        },
                    )),
                    _ => unreachable!(),
                });
            }

            Ok(checks)
        }
        _ => ctx.error(ErrorKind::IncompatibleShapes(
            lhs_info.type_name(),
            rhs_info.type_name(),
        ))?,
    }
}

fn try_match_shapes<'a>(
    func_name: &'static str,
    (lhs, lhs_info): (&mut MirValue, &mut &'a ValueInfo),
    (rhs, rhs_info): (&mut MirValue, &mut &'a ValueInfo),
    tr: &'a FunctionTranslation,
    ctx: AnalyzeContext,
) -> Result<(), Error> {
    for check in try_match_shapes_rec(func_name, lhs_info, rhs_info, ctx)? {
        match check {
            Either::Left(op) => {
                [(*lhs, *lhs_info), (*rhs, *rhs_info)] = tr.add_node(
                    op.into(),
                    [lhs_info.clone(), rhs_info.clone()],
                    [*lhs, *rhs],
                );
            }
            Either::Right((side, op)) => {
                let (val, val_info) =
                    side.select((&mut *lhs, &mut *lhs_info), (&mut *rhs, &mut *rhs_info));
                [(*val, *val_info)] = tr.add_node(op.into(), [val_info.clone()], [*val]);
            }
        }
    }
    Ok(())
}

/// Must only be invoked after calling `try_match_shapes`
#[allow(clippy::too_many_lines)]
fn pervasive_dyadic_rec(
    lhs_info: &ValueInfo,
    rhs_info: &ValueInfo,
    scalar_func: impl Fn(types::ScalarInfo, types::ScalarInfo) -> Result<types::ScalarInfo, Error>
    + Clone,
) -> Result<ValueInfo, Error> {
    use ValueInfo as V;
    let output_info = match (lhs_info, rhs_info, Side::Left, Side::Right) {
        (V::Scalar(l), V::Scalar(r), ..) => V::Scalar(scalar_func(*l, *r)?),
        (V::Array(array), scalar_info @ V::Scalar(_), array_side, _)
        | (scalar_info @ V::Scalar(_), V::Array(array), _, array_side) => match &**array {
            types::ArrayInfo::Known {
                element_type,
                value,
            } => {
                let (lhs_element_type, rhs_element_type) =
                    array_side.place_first(element_type, scalar_info);
                let element_type =
                    pervasive_dyadic_rec(lhs_element_type, rhs_element_type, scalar_func.clone())?;
                let value = types::ArrayValue {
                    shape: value.shape.clone(),
                    data: value
                        .data
                        .iter()
                        .map(|x| {
                            let (lhs_info, rhs_info) = array_side.place_first(x, scalar_info);
                            pervasive_dyadic_rec(lhs_info, rhs_info, scalar_func.clone())
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                };
                ValueInfo::Array(Box::new(types::ArrayInfo::Known {
                    element_type,
                    value,
                }))
            }
            types::ArrayInfo::Ranked {
                element_type,
                shape,
            } => {
                let (lhs_element_type, rhs_element_type) =
                    array_side.place_first(element_type, scalar_info);
                let element_type =
                    pervasive_dyadic_rec(lhs_element_type, rhs_element_type, scalar_func.clone())?;
                ValueInfo::Array(Box::new(types::ArrayInfo::Ranked {
                    element_type,
                    shape: shape.clone(),
                }))
            }
            types::ArrayInfo::Unranked {
                element_type,
                shape_prefix,
                shape_suffix,
            } => {
                let (lhs_element_type, rhs_element_type) =
                    array_side.place_first(element_type, scalar_info);
                let element_type =
                    pervasive_dyadic_rec(lhs_element_type, rhs_element_type, scalar_func.clone())?;
                ValueInfo::Array(Box::new(types::ArrayInfo::Unranked {
                    element_type,
                    shape_prefix: shape_prefix.clone(),
                    shape_suffix: shape_suffix.clone(),
                }))
            }
        },
        (V::Map(map), scalar_info @ V::Scalar(_), map_side, _)
        | (scalar_info @ V::Scalar(_), V::Map(map), _, map_side) => {
            let (lhs_element_type, rhs_element_type) =
                map_side.place_first(&map.value_type, scalar_info);
            let value_type =
                pervasive_dyadic_rec(lhs_element_type, rhs_element_type, scalar_func.clone())?;
            ValueInfo::Map(Box::new(types::MapInfo {
                key_type: map.key_type.clone(),
                value_type,
            }))
        }
        (ValueInfo::Array(l_array), ValueInfo::Array(r_array), ..) => {
            let lhs_shape = l_array.sym_shape().unwrap();
            let rhs_shape = r_array.sym_shape().unwrap();
            let mut new_shape = Vec::new();
            for eob in lhs_shape.iter().zip_longest(rhs_shape.iter()) {
                match eob {
                    itertools::EitherOrBoth::Both(l_ax, r_ax) => {
                        if let Some((l, r)) = l_ax.as_const().zip(r_ax.as_const()) {
                            if l == 1 {
                                new_shape.push(r_ax.clone());
                            } else if r == 1 {
                                new_shape.push(l_ax.clone());
                            } else {
                                assert_eq!(l, r);
                                new_shape.push(l.into());
                            }
                            continue;
                        }
                        // NOTE: Could do some equivalence class nonsense to keep track of what axis equalities have already been checked and potentially omit checks later, though it would probably be a lot more work than it's worth
                        if r_ax.as_const().is_some() {
                            new_shape.push(r_ax.clone());
                        } else {
                            new_shape.push(l_ax.clone());
                        }
                    }
                    itertools::EitherOrBoth::Left(ax) | itertools::EitherOrBoth::Right(ax) => {
                        new_shape.push(ax.clone());
                    }
                }
            }

            let element_type =
                pervasive_dyadic_rec(l_array.element_type(), r_array.element_type(), scalar_func)?;

            ValueInfo::Array(Box::new(types::ArrayInfo::Ranked {
                element_type,
                shape: new_shape,
            }))
        }
        _ => todo!(),
    };

    Ok(output_info)
}

fn pervasive_dyadic(
    func_name: &'static str,
    prim: mir::Prim,
    mut lhs: MirValue,
    mut rhs: MirValue,
    tr: &FunctionTranslation,
    ctx: AnalyzeContext,
    scalar_func: impl Fn(types::ScalarInfo, types::ScalarInfo) -> Result<types::ScalarInfo, Error>
    + Clone,
) -> Result<MirValue, Error> {
    let [mut lhs_info, mut rhs_info] = tr.infos([lhs, rhs]);
    try_match_types(
        func_name,
        (&mut lhs, &mut lhs_info),
        (&mut rhs, &mut rhs_info),
        tr,
        ctx,
    )?;
    try_match_shapes(
        func_name,
        (&mut lhs, &mut lhs_info),
        (&mut rhs, &mut rhs_info),
        tr,
        ctx,
    )?;

    let output_info = pervasive_dyadic_rec(lhs_info, rhs_info, scalar_func)?;

    let [(output, _)] = tr.add_node(prim.into(), [output_info], [lhs, rhs]);

    Ok(output)
}

macro_rules! matching_type_func {
    (
        $name:ident, $name_str:literal, $prim:path;
        $(
            $pattern:pat => $output:expr,
        )*
    ) => {
        pub fn $name(
            lhs: MirValue,
            rhs: MirValue,
            tr: &FunctionTranslation,
            ctx: AnalyzeContext,
        ) -> Result<MirValue, Error> {
            pervasive_dyadic(
                $name_str,
                $prim.into(),
                lhs,
                rhs,
                tr,
                ctx,
                |l, r| {
                    Ok(match (l, r) {
                        $(
                            $pattern => $output,
                        )*
                        _ => ctx.error(ErrorKind::IncompatibleTypes(
                            $name_str,
                            l.type_name().into(),
                            r.type_name().into(),
                        ))?,
                    })
                }
            )
        }
    };
}

matching_type_func! {
    equals, "equals", Pr::Eq;
    (S::Bool(l), S::Bool(r)) => S::Bool(l.zip(r).map(|(l, r)| l == r)),
    (S::Int(l), S::Int(r)) => S::Bool(l.zip(r).map(|(l, r)| l == r)),
    (S::Float(l), S::Float(r)) => S::Bool(l.zip(r).map(|(l, r)| l == r)),
    (S::Char(l), S::Char(r)) => S::Bool(l.zip(r).map(|(l, r)| l == r)),
}

matching_type_func! {
    not_equals, "not equals", Pr::Ne;
    (S::Bool(l), S::Bool(r)) => S::Bool(l.zip(r).map(|(l, r)| l != r)),
    (S::Int(l), S::Int(r)) => S::Bool(l.zip(r).map(|(l, r)| l != r)),
    (S::Float(l), S::Float(r)) => S::Bool(l.zip(r).map(|(l, r)| l != r)),
    (S::Char(l), S::Char(r)) => S::Bool(l.zip(r).map(|(l, r)| l != r)),
}

matching_type_func! {
    less_than, "less than", Pr::Lt;
    (S::Bool(l), S::Bool(r)) => S::Bool(l.zip(r).map(|(l, r)| l && !r)),
    (S::Int(l), S::Int(r)) => S::Bool(l.zip(r).map(|(l, r)| l > r)),
    (S::Float(l), S::Float(r)) => S::Bool(l.zip(r).map(|(l, r)| l > r)),
    (S::Char(l), S::Char(r)) => S::Bool(l.zip(r).map(|(l, r)| l > r)),
}

matching_type_func! {
    less_or_equal, "less or equal", Pr::Le;
    (S::Bool(l), S::Bool(r)) => S::Bool(l.zip(r).map(|(l, r)| l || !r)),
    (S::Int(l), S::Int(r)) => S::Bool(l.zip(r).map(|(l, r)| l >= r)),
    (S::Float(l), S::Float(r)) => S::Bool(l.zip(r).map(|(l, r)| l >= r)),
    (S::Char(l), S::Char(r)) => S::Bool(l.zip(r).map(|(l, r)| l >= r)),
}

matching_type_func! {
    greater_than, "greater than", Pr::Gt;
    (S::Bool(l), S::Bool(r)) => S::Bool(l.zip(r).map(|(l, r)| !l && r)),
    (S::Int(l), S::Int(r)) => S::Bool(l.zip(r).map(|(l, r)| l < r)),
    (S::Float(l), S::Float(r)) => S::Bool(l.zip(r).map(|(l, r)| l < r)),
    (S::Char(l), S::Char(r)) => S::Bool(l.zip(r).map(|(l, r)| l < r)),
}

matching_type_func! {
    greater_or_equal, "greater or equal", Pr::Ge;
    (S::Bool(l), S::Bool(r)) => S::Bool(l.zip(r).map(|(l, r)| !l || r)),
    (S::Int(l), S::Int(r)) => S::Bool(l.zip(r).map(|(l, r)| l <= r)),
    (S::Float(l), S::Float(r)) => S::Bool(l.zip(r).map(|(l, r)| l <= r)),
    (S::Char(l), S::Char(r)) => S::Bool(l.zip(r).map(|(l, r)| l <= r)),
}

macro_rules! float_funcs {
    ($($name:ident, $name_str:literal, $prim:path, $func:expr;)*) => {
        $(
            pub fn $name(
                lhs: MirValue,
                rhs: MirValue,
                tr: &FunctionTranslation,
                ctx: AnalyzeContext,
            ) -> Result<MirValue, Error> {
                let func: fn(f64, f64) -> f64 = $func;
                pervasive_dyadic(
                    $name_str,
                    $prim.into(),
                    lhs,
                    rhs,
                    tr,
                    ctx,
                    |l, r| {
                        let l = match try_match_scalar_types(l, S::Float(None)) {
                            ScalarTypeMatch::Identical => l,
                            ScalarTypeMatch::Matching(Side::Left, _, to) => to,
                            _ => ctx.error(ErrorKind::ExpectedNumber($name_str, l.type_name().into()))?,
                        };
                        let r = match try_match_scalar_types(r, S::Float(None)) {
                            ScalarTypeMatch::Identical => r,
                            ScalarTypeMatch::Matching(Side::Left, _, to) => to,
                            _ => ctx.error(ErrorKind::ExpectedNumber($name_str, r.type_name().into()))?,
                        };
                        let (S::Float(l), S::Float(r)) = (l, r) else {
                            unreachable!()
                        };
                        Ok(S::Float(l.zip(r).map(|(l, r)| func(l, r))))
                    },
                )
            }
        )*
    };
}

use uiua::{ImplPrimitive as Ip, Primitive as Pr};
float_funcs! {
    divide, "divide", Pr::Div, |l, r| r / l;
    atangent, "atangent", Pr::Atan, |l, r| l.atan2(r);

    root, "root", Ip::Root, |l, r| r.powf(l.recip());
}
