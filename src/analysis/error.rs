use std::collections::HashMap;
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
    #[error("Cannot take the square root of a character")]
    SqrtChar,
}

impl FancyError {
    pub fn eprint(&self) {
        use ariadne::{ColorGenerator, Label, Report, ReportKind, Source};

        let mut colors = ColorGenerator::new();

        match &self.kind {
            ErrorKind::UiuaValue(err) => {
                let (source_path, source, range) = span_to_ariadne(&self.span, &self.files);
                Report::build(ReportKind::Error, (&source_path, range.clone()))
                    .with_message(err)
                    .with_label(
                        Label::new((&source_path, range.clone()))
                            .with_message(err)
                            .with_color(colors.next()),
                    )
                    .finish()
                    .eprint((&source_path, Source::from(source)))
                    .unwrap();
            }
            ErrorKind::SqrtChar => {
                let (source_path, source, range) = span_to_ariadne(&self.span, &self.files);
                let (input_source_path, _input_source, input_range) =
                    span_to_ariadne(&self.input_spans[0], &self.files);

                Report::build(ReportKind::Error, (&source_path, range.clone()))
                    .with_message(&self.kind)
                    .with_label(
                        Label::new((&source_path, range))
                            .with_message("Square root expects numbers")
                            .with_color(colors.next()),
                    )
                    .with_label(
                        Label::new((&input_source_path, input_range))
                            .with_message("Characters produced here")
                            .with_color(colors.next())
                            .with_order(-1),
                    )
                    .finish()
                    .eprint((&source_path, Source::from(source)))
                    .unwrap();
            }
        }
    }
}

fn span_to_ariadne<'a>(
    span: &'a uiua::Span,
    files: &'a HashMap<PathBuf, String>,
) -> (std::borrow::Cow<'a, str>, &'a str, std::ops::Range<usize>) {
    span.code_ref()
        .and_then(|span| code_span_to_ariadne(span, files))
        .unwrap_or_else(|| (std::borrow::Cow::default(), "", 0..0))
}

fn code_span_to_ariadne<'a>(
    span: &'a uiua::CodeSpan,
    files: &'a HashMap<PathBuf, String>,
) -> Option<(std::borrow::Cow<'a, str>, &'a str, std::ops::Range<usize>)> {
    match &span.src {
        uiua::InputSrc::File(path) => Some((
            path.to_string_lossy(),
            &*files[&**path],
            span.start.char_pos as usize..span.end.char_pos as usize,
        )),
        uiua::InputSrc::Str(_) => None,
        uiua::InputSrc::Macro(code_span) => code_span_to_ariadne(code_span, files),
        uiua::InputSrc::Literal(string) => Some((
            std::borrow::Cow::default(),
            string,
            0..string.chars().count(),
        )),
    }
}
