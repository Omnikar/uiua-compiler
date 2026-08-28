#![warn(clippy::pedantic)]

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(about = "Uiua native compiler")]
struct Args {
    filepath: String,
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
            .load_file(path)?
            .finish(),
        Some(ext) if ext == "uasm" => {
            let uasm_text = std::fs::read_to_string(path)?;
            uiua::Assembly::from_uasm(&uasm_text)?
        }
        _ => return Err(ProgramError::WrongFileType),
    };

    Ok(())
}
