use ariadne::{ColorGenerator, Label, Report, ReportKind, Source};
use itertools::Itertools;
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
    pub kind: ErrorKind,
}

#[derive(thiserror::Error, Debug)]
pub enum ErrorKind {
    #[error("{0}")]
    UiuaValue(UiuaValueError),
    #[error("Cannot not character")]
    NotChar,
    #[error("Cannot take the reciprocal of a character")]
    RecipChar,
    #[error("Cannot take the square root of a character")]
    SqrtChar,
}

impl FancyError {
    fn simple<'a>(
        &self,
        parent_msg: impl ToString,
        source_msg: impl ToString,
        input_msgs: impl IntoIterator<Item = &'a str>,
    ) {
        let (source_path, source, range) = span_to_ariadne(&self.span, &self.files);
        let (input_source_paths, _input_sources, input_ranges): (Vec<_>, Vec<_>, Vec<_>) = self
            .input_spans
            .iter()
            .map(|input_span| span_to_ariadne(input_span, &self.files))
            .multiunzip();

        let mut colors = ColorGenerator::new();

        let mut builder = Report::build(ReportKind::Error, (&source_path, range.clone()))
            .with_message(parent_msg)
            .with_label(
                Label::new((&source_path, range))
                    .with_message(source_msg)
                    .with_color(colors.next()),
            );
        for (i, msg) in input_msgs.into_iter().enumerate() {
            builder.add_label(
                Label::new((&input_source_paths[i], input_ranges[i].clone()))
                    .with_message(msg)
                    .with_color(colors.next())
                    .with_order(-1),
            );
        }
        builder
            .finish()
            .eprint((&source_path, Source::from(source)))
            .unwrap();
    }

    fn expected_numbers(&self, err: impl ToString, name: &str) {
        self.simple(
            err,
            format!("{name} expects numbers"),
            ["Characters produced here"],
        );
    }

    pub fn eprint(&self) {
        match &self.kind {
            ErrorKind::UiuaValue(err) => self.simple(err, err, []),
            err @ ErrorKind::NotChar => self.expected_numbers(err, "Not"),
            err @ ErrorKind::RecipChar => self.expected_numbers(err, "Reciprocal"),
            err @ ErrorKind::SqrtChar => self.expected_numbers(err, "Square root"),
        }
    }
}

fn span_to_ariadne<'a>(
    span: &'a uiua::Span,
    files: &'a HashMap<PathBuf, String>,
) -> (Cow<'a, str>, &'a str, Range<usize>) {
    span.code_ref()
        .and_then(|span| code_span_to_ariadne(span, files))
        .unwrap_or_else(|| (Cow::default(), "", 0..0))
}

fn code_span_to_ariadne<'a>(
    span: &'a uiua::CodeSpan,
    files: &'a HashMap<PathBuf, String>,
) -> Option<(Cow<'a, str>, &'a str, Range<usize>)> {
    match &span.src {
        uiua::InputSrc::File(path) => Some((
            path.to_string_lossy(),
            &*files[&**path],
            span.start.char_pos as usize..span.end.char_pos as usize,
        )),
        uiua::InputSrc::Str(_) => None,
        uiua::InputSrc::Macro(code_span) => code_span_to_ariadne(code_span, files),
        uiua::InputSrc::Literal(string) => {
            Some((Cow::default(), string, 0..string.chars().count()))
        }
    }
}
