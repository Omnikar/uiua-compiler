use ariadne::{Color, Fmt, Label, Report, ReportKind};
use itertools::Itertools;
use rand::{RngExt, SeedableRng};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use super::UiuaValueError;

#[derive(thiserror::Error, Debug)]
#[error("{span}: {kind}")]
pub struct FancyError {
    pub files: Rc<HashMap<PathBuf, String>>,
    pub span: uiua::Span,
    pub input_spans: Vec<uiua::Span>,
    pub call_spans: Vec<uiua::Span>,
    pub kind: ErrorKind,
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum ErrorKind {
    #[error("{0}")]
    UiuaValue(UiuaValueError),
    #[error("Could not infer input rank")]
    Unranked(&'static str),
    #[error("Cannot take the {0} of a character")]
    ExpectedNumber(&'static str, Rc<str>),
    #[error("Incompatible types: {1} and {2}")]
    IncompatibleTypes(&'static str, Rc<str>, Rc<str>),
    #[error("Incompatible shapes: {0} and {1}")]
    IncompatibleShapes(Rc<str>, Rc<str>),
    #[error("Mismatched row counts: {0} and {2}")]
    MismatchedRowCounts(isize, usize, isize, usize),
}

trait Msg {
    fn fmt(&self, color: Color) -> String;
}
impl Msg for &str {
    fn fmt(&self, _: Color) -> String {
        self.to_string()
    }
}
impl Msg for String {
    fn fmt(&self, _: Color) -> String {
        self.clone()
    }
}
impl<F: FnOnce(Color) -> String + Clone> Msg for F {
    fn fmt(&self, color: Color) -> String {
        self.clone()(color)
    }
}
impl Msg for Box<dyn Msg> {
    fn fmt(&self, color: Color) -> String {
        (**self).fmt(color)
    }
}

impl FancyError {
    fn simple(
        &self,
        parent_msg: impl ToString,
        source_msg: impl Msg,
        input_msgs: impl IntoIterator<Item = impl Msg>,
    ) {
        self.with_input_msg_idxs(parent_msg, source_msg, input_msgs.into_iter().enumerate());
    }

    fn with_input_msg_idxs(
        &self,
        parent_msg: impl ToString,
        source_msg: impl Msg,
        input_msgs: impl IntoIterator<Item = (usize, impl Msg)>,
    ) {
        let (source_path, source, range) = span_to_ariadne(&self.span, &self.files);
        let (input_source_paths, input_sources, input_ranges): (Vec<_>, Vec<_>, Vec<_>) = self
            .input_spans
            .iter()
            .map(|input_span| span_to_ariadne(input_span, &self.files))
            .multiunzip();

        let mut colors = PastelGenerator::new();

        let color = colors.next();
        let mut builder = Report::build(ReportKind::Error, (source_path.clone(), range.clone()))
            .with_message(parent_msg)
            .with_label(
                Label::new((source_path.clone(), range))
                    .with_message(source_msg.fmt(color))
                    .with_color(color),
            );
        let input_msgs = input_msgs.into_iter().collect_vec();
        for (i, msg) in input_msgs.into_iter().rev() {
            let color = colors.next();
            builder.add_label(
                Label::new((input_source_paths[i].clone(), input_ranges[i].clone()))
                    .with_message(msg.fmt(color))
                    .with_color(color)
                    .with_order(-1),
            );
        }

        let call_ariadne_spans = self
            .call_spans
            .iter()
            .map(|call_span| span_to_ariadne(call_span, &self.files))
            .collect_vec();
        for (call_source_path, _, call_range) in &call_ariadne_spans {
            builder.add_label(
                Label::new((call_source_path.clone(), call_range.clone()))
                    .with_message("In this function call")
                    .with_color(Color::BrightCyan)
                    .with_order(-2),
            );
        }
        let (call_source_paths, call_sources, _): (Vec<_>, Vec<_>, Vec<_>) =
            call_ariadne_spans.into_iter().multiunzip();

        let sources = input_source_paths
            .into_iter()
            .chain(call_source_paths)
            .chain([source_path])
            .zip_eq(
                input_sources
                    .into_iter()
                    .chain(call_sources)
                    .chain([source]),
            );
        let mut cache = ariadne::sources(sources);

        builder.finish().eprint(&mut cache).unwrap();
    }

    pub fn eprint(&self) {
        use ErrorKind as Ek;
        match self.kind.clone() {
            Ek::UiuaValue(err) => self.simple(err, err.to_string(), [] as [&str; 0]),
            Ek::Unranked(name) => self.simple(
                &self.kind,
                |c| {
                    format!(
                        "{} must know the rank of its inputs",
                        capitalize(name).fg(c)
                    )
                },
                ["Could not infer rank"],
            ),
            Ek::ExpectedNumber(name, input) => self.simple(
                &self.kind,
                |c| format!("{} expects numbers", capitalize(name).fg(c)),
                [|c| format!("{} produced here", capitalize(&input).fg(c))],
            ),
            Ek::IncompatibleTypes(name, left, right) => self.simple(
                &self.kind,
                |c| format!("{} cannot process these types", capitalize(name).fg(c)),
                [
                    Box::new(move |c| format!("{} produced here", capitalize(&left).fg(c)))
                        as Box<dyn Msg>,
                    Box::new(move |c| format!("{} produced here", capitalize(&right).fg(c))),
                ],
            ),
            Ek::IncompatibleShapes(left, right) => self.simple(
                &self.kind,
                "Pervasive operations require matching shapes",
                [
                    Box::new(move |c| format!("{} produced here", capitalize(&left).fg(c)))
                        as Box<dyn Msg>,
                    Box::new(move |c| format!("{} produced here", capitalize(&right).fg(c))),
                ],
            ),
            Ek::MismatchedRowCounts(left_len, left_i, right_len, right_i) => self
                .with_input_msg_idxs(
                    &self.kind,
                    "Row counts must match",
                    [
                        (
                            left_i,
                            Box::new(move |c| {
                                format!(
                                    "{} argument produced here",
                                    format!("{left_len} row").fg(c)
                                )
                            }) as Box<dyn Msg>,
                        ),
                        (
                            right_i,
                            Box::new(move |c| {
                                format!(
                                    "{} argument produced here",
                                    format!("{right_len} row").fg(c)
                                )
                            }),
                        ),
                    ],
                ),
        }
    }
}

fn capitalize(s: &str) -> Cow<'_, str> {
    let mut chars = s.chars();
    if let Some(c) = chars.next()
        && c.is_lowercase()
    {
        c.to_uppercase().chain(chars).collect::<String>().into()
    } else {
        s.into()
    }
}

fn span_to_ariadne<'a>(
    span: &'a uiua::Span,
    files: &'a HashMap<PathBuf, String>,
) -> (Rc<str>, &'a str, Range<usize>) {
    span.code_ref()
        .and_then(|span| code_span_to_ariadne(span, files))
        .unwrap_or_else(|| (Rc::default(), "", 0..0))
}

fn code_span_to_ariadne<'a>(
    span: &'a uiua::CodeSpan,
    files: &'a HashMap<PathBuf, String>,
) -> Option<(Rc<str>, &'a str, Range<usize>)> {
    match &span.src {
        uiua::InputSrc::File(path) => Some((
            path.to_string_lossy().into(),
            &*files[&**path],
            span.start.char_pos as usize..span.end.char_pos as usize,
        )),
        uiua::InputSrc::Str(_) => None,
        uiua::InputSrc::Macro(code_span) => code_span_to_ariadne(code_span, files),
        uiua::InputSrc::Literal(string) => Some((Rc::default(), string, 0..string.chars().count())),
    }
}

struct PastelGenerator {
    state: rand::rngs::StdRng,
}

impl PastelGenerator {
    fn new() -> Self {
        Self {
            state: rand::rngs::StdRng::from_seed([0; 32]),
        }
    }

    fn next(&mut self) -> Color {
        let hue = self.state.random_range(0.0..std::f32::consts::TAU);
        let saturation = self.state.random_range(0.25..0.7);
        let lightness = self.state.random_range(0.75..0.9);
        color_from_hsl(hue, saturation, lightness)
    }
}

fn color_from_hsl(hue: f32, saturation: f32, lightness: f32) -> Color {
    let chroma = (1.0 - f32::abs(2.0 * lightness - 1.0)) * saturation;
    let hue_div = hue / std::f32::consts::FRAC_PI_3;
    let secondary = chroma * (1.0 - f32::abs(hue_div.rem_euclid(2.0) - 1.0));
    let lightness_match = lightness - chroma / 2.0;

    let components = match hue_div {
        0.0..1.0 => [chroma, secondary, 0.0],
        1.0..2.0 => [secondary, chroma, 0.0],
        2.0..3.0 => [0.0, chroma, secondary],
        3.0..4.0 => [0.0, secondary, chroma],
        4.0..5.0 => [secondary, 0.0, chroma],
        5.0..6.0 => [chroma, 0.0, secondary],
        _ => unreachable!(),
    };

    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let [r, g, b] = components.map(|v| f32::round((v + lightness_match) * 255.0) as u8);

    Color::Rgb(r, g, b)
}
