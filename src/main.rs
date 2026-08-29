#![warn(clippy::pedantic)]
#![allow(clippy::items_after_statements)]

mod data_flow;
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
    #[arg(long, value_enum, default_value_t = EmitFormat::Dot)]
    emit: EmitFormat,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum EmitFormat {
    Dot,
    LlvmIr,
    Executable,
}

#[derive(thiserror::Error, Debug)]
enum ProgramError {
    #[error("{0}")]
    ClapError(#[from] clap::Error),

    #[error("error: Expected a `.ua` or `.uasm` file")]
    WrongFileType,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("{0}")]
    InterpreterError(#[from] uiua::UiuaError),

    #[error("{0}")]
    DataFlowError(#[from] data_flow::Error),

    #[error("error: {0}")]
    Other(String),
}

impl From<String> for ProgramError {
    fn from(message: String) -> Self {
        ProgramError::Other(message)
    }
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(ProgramError::ClapError(e)) => e.exit(),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<(), ProgramError> {
    let args = Args::try_parse()?;

    let path = PathBuf::from(args.filepath);
    let uasm = match path.extension() {
        Some(ext) if ext == "ua" => uiua::Compiler::with_backend(uiua::NativeSys)
            .load_file(&path)?
            .finish(),
        Some(ext) if ext == "uasm" => {
            let uasm_text = std::fs::read_to_string(&path)?;
            uiua::Assembly::from_uasm(&uasm_text)?
        }
        _ => return Err(ProgramError::WrongFileType),
    };

    match args.emit {
        EmitFormat::Dot => {
            let hir = data_flow::construct_hir(&uasm)?;
            let mut output: Box<dyn Write> = if let Some(filename) = &args.output {
                Box::new(std::fs::File::create(filename)?)
            } else {
                Box::new(std::io::stdout())
            };
            for binding in hir.bindings {
                let dot = petgraph::dot::Dot::new(&binding.func.graph);
                let mut dot_s = format!("{dot:?}")
                    .strip_prefix("digraph {\n")
                    .unwrap()
                    .to_owned();
                dot_s.insert_str(
                    0,
                    r#"digraph {
    node [shape=box]
    node [fontname="Uiua386"]
    edge [fontname="Uiua386"]
"#,
                );
                write!(output, "{dot_s}").unwrap();
            }
        }
        _ => todo!(),
    }

    Ok(())
}
