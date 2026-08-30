#![warn(clippy::pedantic)]
#![allow(
    clippy::items_after_statements,
    clippy::match_wildcard_for_single_variants,
    clippy::zero_sized_map_values
)]

mod data_flow;
mod generic_ir;
mod hir;

use clap::Parser;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(about = "Uiua native compiler")]
struct Args {
    filepath: String,
    #[arg(short)]
    output: Option<String>,
    // TODO: Eventually change the default to executable
    // TODO: Infer based on output filename extension when not provided
    #[arg(long, value_enum, default_value_t = EmitFormat::Hir)]
    emit: EmitFormat,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
enum EmitFormat {
    Uasm,
    Dot,
    Hir,
    LlvmIr,
    Executable,
}
impl std::fmt::Display for EmitFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        clap::ValueEnum::to_possible_value(self)
            .unwrap()
            .get_name()
            .fmt(f)
    }
}

#[derive(thiserror::Error, Debug)]
enum ProgramError {
    #[error("{0}")]
    ClapError(#[from] clap::Error),

    #[error("Unknown file type")]
    WrongFileType,

    #[error("Cannot convert from {0} to {1}")]
    InvalidConversion(EmitFormat, EmitFormat),

    #[error("{0}")]
    IoError(#[from] std::io::Error),

    #[error("{0}")]
    InterpreterError(#[from] uiua::UiuaError),

    #[error("{0}")]
    DataFlowError(#[from] data_flow::Error),

    #[error("{0}")]
    DeserializeError(#[from] ron::error::SpannedError),

    #[error("{0}")]
    Other(String),
}

impl From<String> for ProgramError {
    fn from(message: String) -> Self {
        ProgramError::Other(message)
    }
}

// As clippy warns, the data contained in these variants are quite large and variable in size, so they are stored behind pointers
#[derive(Debug)]
enum LoweringState {
    Uasm(Box<uiua::Assembly>),
    Hir(Box<hir::Hir>),
    Dot(String),
}
impl LoweringState {
    fn cur_format(&self) -> EmitFormat {
        match self {
            Self::Uasm(_) => EmitFormat::Uasm,
            Self::Hir(_) => EmitFormat::Hir,
            Self::Dot(_) => EmitFormat::Dot,
        }
    }

    fn convert_to(self, format: EmitFormat) -> Result<Self, ProgramError> {
        use EmitFormat as Ef;
        use LoweringState as Ls;
        Ok(match (&self, format) {
            _ if self.cur_format() == format => self,
            (Ls::Uasm(uasm), Ef::Hir) => Ls::Hir(Box::new(data_flow::construct_hir(uasm)?)),
            (Ls::Uasm(_), Ef::Dot) => self.convert_to(Ef::Hir)?.convert_to(Ef::Dot)?,
            (Ls::Hir(hir), Ef::Dot) => {
                let mut result = String::new();
                for binding in &hir.bindings {
                    let dot = petgraph::dot::Dot::new(&binding.func.graph);
                    let mut dot_s = format!("{dot:?}")
                        .strip_prefix("digraph {\n")
                        .unwrap()
                        .to_owned();
                    dot_s = format!(
                        r#"digraph {{
    node [shape=box]
    node [fontname="Uiua386"]
    edge [fontname="Uiua386"]
    label = "{}"
    labelloc = "t"
{dot_s}"#,
                        binding.func_id
                    );
                    result.push_str(&dot_s);
                }
                Self::Dot(result)
            }
            _ => return Err(ProgramError::InvalidConversion(self.cur_format(), format)),
        })
    }

    fn write_output(&self, output: &mut dyn Write) -> Result<(), ProgramError> {
        match self {
            LoweringState::Uasm(uasm) => {
                writeln!(output, "{}", uasm.to_uasm())?;
            }
            LoweringState::Hir(hir) => {
                writeln!(output, "{hir}")?;
            }
            LoweringState::Dot(s) => {
                writeln!(output, "{s}")?;
            }
        }
        Ok(())
    }
}

fn run() -> Result<(), ProgramError> {
    let args = Args::try_parse()?;

    let path = PathBuf::from(args.filepath);
    let mut state = match path.extension() {
        Some(ext) if ext == "ua" => LoweringState::Uasm(Box::new(
            uiua::Compiler::with_backend(uiua::NativeSys)
                .pre_eval_mode(uiua::PreEvalMode::Lazy)
                .load_file(&path)?
                .finish(),
        )),
        Some(ext) if ext == "uasm" => {
            let uasm_text = std::fs::read_to_string(&path)?;
            LoweringState::Uasm(Box::new(uiua::Assembly::from_uasm(&uasm_text)?))
        }
        Some(ext) if ext == "hir" => {
            let hir_text = std::fs::read_to_string(&path)?;
            let hir: hir::Hir = ron::from_str(&hir_text)?;
            LoweringState::Hir(Box::new(hir))
        }
        _ => return Err(ProgramError::WrongFileType),
    };

    state = state.convert_to(args.emit)?;

    let mut output: Box<dyn Write> = if let Some(filename) = &args.output {
        Box::new(std::fs::File::create(filename)?)
    } else if args.emit == EmitFormat::Executable {
        let filename = path.file_stem().unwrap();
        Box::new(std::fs::File::create(filename)?)
    } else {
        Box::new(std::io::stdout())
    };

    state.write_output(&mut output)
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(ProgramError::ClapError(e)) => e.exit(),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
