use itertools::Itertools;

use super::{
    AnalyzeContext, Error, ErrorKind, FunctionTranslator, Side, TirValue, ValueInfo, analysis, tir,
    uir,
};

pub fn rows(
    uir_func: &uir::Function,
    inputs: &[TirValue],
    tr: &FunctionTranslator,
    ctx: AnalyzeContext,
    func_input_spans: Vec<&uiua::Span>,
) -> Result<Vec<TirValue>, Error> {
    let input_infos = tr.infos_dyn(inputs).collect_vec();

    let mut inputs: std::borrow::Cow<[TirValue]> = inputs.into();

    let (n_rows, row_infos) = input_infos
        .iter()
        .map(|&input_info| {
            Ok::<_, Error>(match input_info {
                ValueInfo::Array(array_info) => {
                    let (n_rows, row_array_info) = array_info
                        .split_first_axis()
                        .ok_or_else(|| ctx.make_error(ErrorKind::Unranked("rows")))?;
                    (Some(n_rows), row_array_info)
                }
                _ => (None, input_info.clone()),
            })
        })
        .enumerate()
        .try_fold(
            (None::<(tir::Expr, usize)>, Vec::<ValueInfo>::new()),
            |(cur_n_rows, mut cur_row_infos), (new_i, res)| {
                let (n_rows, row_info) = res?;
                let new_n_rows = match (cur_n_rows, n_rows, new_i) {
                    (None, None, _) => None,
                    (Some((n, i)), None, _) | (None, Some(n), i) => Some((n, i)),
                    (Some((n_expr, old_i)), Some(m_expr), _) => Some(
                        match (
                            n_expr.as_const(),
                            m_expr.as_const(),
                            Side::Left,
                            Side::Right,
                        ) {
                            (Some(n), Some(m), ..) => {
                                if n == m || m == 1 {
                                    (n_expr, old_i)
                                } else if n == 1 {
                                    (m_expr, new_i)
                                } else {
                                    ctx.error(ErrorKind::MismatchedRowCounts(n, old_i, m, new_i))?
                                }
                            }
                            (Some(n), None, _, side) | (None, Some(n), side, _) => {
                                let input_i = side.select(old_i, new_i);
                                if n == 1 {
                                    let inputs = inputs.to_mut();
                                    [(inputs[input_i], _)] = tr.add_node(
                                        tir::TirOp::CheckAxis {
                                            depth: 0,
                                            ax_i: 0,
                                            length: n.cast_unsigned(),
                                        }
                                        .into(),
                                        [input_infos[input_i].clone()],
                                        [inputs[input_i]],
                                    );
                                    (n.into(), side.select(new_i, old_i))
                                } else {
                                    (side.select(n_expr, m_expr), input_i)
                                }
                            }
                            (None, None, ..) => {
                                let inputs = inputs.to_mut();

                                if (n_expr.clone() - m_expr).as_const().is_none_or(|x| x != 0) {
                                    [(inputs[old_i], _), (inputs[new_i], _)] = tr.add_node(
                                        tir::TirOp::CheckAxes {
                                            lhs_depth: 0,
                                            lhs_ax_i: 0,
                                            rhs_depth: 0,
                                            rhs_ax_i: 0,
                                        }
                                        .into(),
                                        [input_infos[old_i].clone(), input_infos[new_i].clone()],
                                        [inputs[old_i], inputs[new_i]],
                                    );
                                }

                                (n_expr, old_i)
                            }
                        },
                    ),
                };
                cur_row_infos.push(row_info);
                Ok((new_n_rows, cur_row_infos))
            },
        )?;

    let tir_func =
        analysis::monomorphize_and_analyze(uir_func, row_infos, tr.uir, tr.tir, func_input_spans)?;

    if let Some((n_rows, _)) = n_rows {
        let output_infos = tir_func
            .meta
            .outputs
            .iter()
            .map(|output_info| output_info.with_prepended_first_axis(n_rows.clone()))
            .collect_vec();
        let n_outputs = output_infos.len();
        let outputs = tr.add_node_dyn(
            tir::Node::ModPrim(uiua::Primitive::Rows.into(), [tir_func].into()),
            output_infos,
            inputs.iter().copied(),
            n_outputs,
        );
        Ok(outputs.into_iter().map(|(val, _)| val).collect())
    } else {
        todo!("Inline the function when the inputs are scalars")
    }
}
