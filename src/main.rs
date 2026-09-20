#![warn(clippy::pedantic, clippy::allow_attributes)]
#![expect(
    clippy::items_after_statements,
    clippy::match_wildcard_for_single_variants,
    clippy::needless_pass_by_value
)]

mod generic_ir;
mod uir;
mod tir;

/// Simulation of data flow through a program and initial IR construction
mod data_flow;
/// Static analysis of type, rank, and shape
mod analysis;

use clap::Parser;
use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(about = "Uiua native compiler")]
struct Args {
    #[arg(help = "Input file path (- for stdin)")]
    filepath: String,
    #[arg(short, help = "Output file path")]
    output: Option<String>,
    // TODO: Eventually change the default to executable
    // TODO: Infer based on output filename extension when not provided
    #[arg(long, value_enum, default_value_t = EmitFormat::Uir)]
    emit: EmitFormat,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
enum EmitFormat {
    #[value(skip)]
    Ua,
    Uasm,
    Dot,
    Uir,
    Tir,
    LlvmIr,
    Executable,
}
impl std::fmt::Display for EmitFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match clap::ValueEnum::to_possible_value(self) {
            Some(val) => val.get_name().fmt(f),
            None => write!(f, "given format"),
        }
    }
}

#[derive(thiserror::Error, Debug)]
enum ProgramError {
    #[error("{0}")]
    ClapError(#[from] clap::Error),

    #[error("Unknown file type{}", .0.as_ref().map_or_else(String::new, |x| format!(": {x}")))]
    UnknownFileType(Option<String>),

    #[error("Cannot convert from {0} to {1}")]
    InvalidConversion(EmitFormat, EmitFormat),

    #[error("{0}")]
    IoError(#[from] std::io::Error),

    #[error("{0}")]
    InterpreterError(#[from] uiua::UiuaError),

    #[error("{0}")]
    DataFlowError(#[from] data_flow::Error),

    #[error("{0}")]
    AnalysisError(#[from] analysis::Error),

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
    Ua(PathBuf),
    UaStr(String),
    Uasm(Box<uiua::Assembly>),
    Uir(Box<uir::Uir>),
    Tir(Box<tir::Tir>),
    Dot(String),
}
impl LoweringState {
    fn cur_format(&self) -> EmitFormat {
        match self {
            Self::Ua(_) | Self::UaStr(_) => EmitFormat::Ua,
            Self::Uasm(_) => EmitFormat::Uasm,
            Self::Uir(_) => EmitFormat::Uir,
            Self::Tir(_) => EmitFormat::Tir,
            Self::Dot(_) => EmitFormat::Dot,
        }
    }

    fn convert_to(self, format: EmitFormat) -> Result<Self, ProgramError> {
        use EmitFormat as Ef;
        use LoweringState as Ls;
        Ok(match (&self, format) {
            _ if self.cur_format() == format => self,
            (Ls::Ua(ua_path), Ef::Uasm) => Ls::Uasm(Box::new(
                uiua::Compiler::with_backend(uiua::NativeSys)
                    .pre_eval_mode(uiua::PreEvalMode::Lazy)
                    .load_file(ua_path)?
                    .finish(),
            )),
            (Ls::UaStr(ua_text), Ef::Uasm) => Ls::Uasm(Box::new(
                uiua::Compiler::with_backend(uiua::NativeSys)
                    .pre_eval_mode(uiua::PreEvalMode::Lazy)
                    .load_str(ua_text)?
                    .finish(),
            )),
            (Ls::Uasm(uasm), Ef::Uir) => Ls::Uir(Box::new(data_flow::construct_uir(uasm)?)),
            (Ls::Uir(uir), Ef::Dot) => {
                let mut result = String::new();
                for (func, name) in uir
                    .bindings
                    .iter()
                    .map(|binding| (&binding.func, binding.func_id.to_string()))
                    .chain(uir.main.as_ref().map(|(func, _)| (func, "main".into())))
                {
                    let dot = petgraph::dot::Dot::new(&func.graph);
                    let mut dot_s = format!("{dot:?}")
                        .strip_prefix("digraph {\n")
                        .unwrap()
                        .to_owned();
                    dot_s = format!(
                        r#"digraph {{
    node [shape=box]
    node [fontname="Uiua386"]
    edge [fontname="Uiua386"]
    label = "{name}"
    labelloc = "t"
{dot_s}"#
                    );
                    result.push_str(&dot_s);
                }
                Self::Dot(result)
            }
            (Ls::Uir(uir), Ef::Tir) => Ls::Tir(Box::new(analysis::construct_tir(uir)?)),
            (Ls::Ua(_) | Ls::UaStr(_), ef) => self.convert_to(Ef::Uasm)?.convert_to(ef)?,
            (Ls::Uasm(_), ef) => self.convert_to(Ef::Uir)?.convert_to(ef)?,
            (Ls::Uir(_), ef) => self.convert_to(Ef::Tir)?.convert_to(ef)?,
            _ => return Err(ProgramError::InvalidConversion(self.cur_format(), format)),
        })
    }

    fn write_output(&self, output: &mut dyn Write) -> Result<(), ProgramError> {
        use LoweringState as Ls;
        match self {
            Ls::Ua(_) | Ls::UaStr(_) => unreachable!(),
            Ls::Uasm(uasm) => {
                writeln!(output, "{}", uasm.to_uasm())?;
            }
            Ls::Uir(uir) => {
                writeln!(output, "{uir}")?;
            }
            Ls::Tir(tir) => {
                writeln!(output, "{tir}")?;
            }
            Ls::Dot(s) => {
                writeln!(output, "{s}")?;
            }
        }
        Ok(())
    }
}

fn run() -> Result<(), ProgramError> {
    let args = Args::try_parse()?;

    let path = PathBuf::from(&args.filepath);
    let mut state = match path.extension() {
        Some(ext) if ext == "ua" => LoweringState::Ua(path.clone()),
        Some(ext) if ext == "uasm" => {
            let uasm_text = std::fs::read_to_string(&path)?;
            LoweringState::Uasm(Box::new(uiua::Assembly::from_uasm(&uasm_text)?))
        }
        Some(ext) if ext == "uir" => {
            let uir_text = std::fs::read_to_string(&path)?;
            let uir: uir::Uir = ron::from_str(&uir_text)?;
            LoweringState::Uir(Box::new(uir))
        }
        Some(ext) if ext == "tir" => {
            let tir_text = std::fs::read_to_string(&path)?;
            let tir: tir::Tir = ron::from_str(&tir_text)?;
            LoweringState::Tir(Box::new(tir))
        }
        None if args.filepath == "-" => {
            let mut ua_text = String::new();
            std::io::stdin().read_to_string(&mut ua_text)?;
            LoweringState::UaStr(ua_text)
        }
        ext => {
            return Err(ProgramError::UnknownFileType(
                ext.map(|x| x.to_string_lossy().into_owned()),
            ));
        }
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
    let Err(err) = run() else { return };

    match err {
        ProgramError::ClapError(err) => err.exit(),
        ProgramError::AnalysisError(analysis::Error::FancyError(err)) => err.eprint(),
        err => eprintln!("error: {err}"),
    }
    std::process::exit(1);
}
