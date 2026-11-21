#![feature(const_cmp)]
#![feature(const_trait_impl)]
mod repl;
mod user;

use std::fs;
use std::io::{BufWriter, Write as _, stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lexopt::prelude::*;
use rhesus_core::host::WriterHost;
use rhesus_core::{Engine, Session};

type Result<T = ()> = core::result::Result<T, Box<dyn core::error::Error>>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match parse_args()? {
        Command::Help => write!(stdout().lock(), "{HELP}")?,
        Command::Version => writeln!(stdout().lock(), "rhesus {}", env!("CARGO_PKG_VERSION"))?,
        Command::Repl { engine } => repl::start(engine)?,
        Command::Run { file, engine } => {
            let source = read_source(&file)?;
            let mut out = BufWriter::new(stdout().lock());
            let mut session = Session::new(engine);

            let result = session.eval_line(&source, &mut WriterHost(&mut out));

            match result {
                Ok(Some(output)) => writeln!(out, "{output}")?,
                Ok(None) => {}
                Err(err) => {
                    out.flush()?;
                    return Err(err.into());
                }
            }
            out.flush()?;
        }
        Command::Compile { file } => {
            let source = read_source(&file)?;
            let mut out = BufWriter::new(stdout().lock());
            // TODO: Wire up Compiler::compile(&program) and print bytecode
            // instructions
            writeln!(out, "Compiling {} ({} bytes)...", file.display(), source.len())?;
            out.flush()?;
        }
    }

    Ok(())
}

fn read_source(file: &Path) -> Result<String> {
    fs::read_to_string(file)
        .map_err(|err| format!("Failed to read file: {}: {err}", file.display()).into())
}

const HELP: &str = "\
Rhesus - The Monkey programming language interpreter & bytecode VM

USAGE:
    rhesus [SUBCOMMAND] [OPTIONS]

SUBCOMMANDS:
    repl                Start the interactive REPL (default if no subcommand is given)
    run <FILE>          Execute a Monkey script (bytecode VM unless --engine says otherwise)
    eval <FILE>         Execute a Monkey script using the tree-walking interpreter
    compile <FILE>      Compile a Monkey script and print bytecode disassembly

OPTIONS:
    -e, --engine <MODE> Engine for repl and run ('vm' or 'eval') [default: vm]
    -h, --help          Print help information
    -V, --version       Print version information
";

enum Command {
    Help,
    Version,
    Repl { engine: Engine },
    Run { file: PathBuf, engine: Engine },
    Compile { file: PathBuf },
}

fn parse_args() -> Result<Command> {
    let mut parser = lexopt::Parser::from_env();
    let mut subcommand: Option<String> = None;
    let mut file: Option<PathBuf> = None;
    let mut engine = Engine::Vm;

    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => return Ok(Command::Help),
            Short('V') | Long("version") => return Ok(Command::Version),
            Short('e') | Long("engine") => engine = parser.value()?.parse()?,
            Value(val) if subcommand.is_none() => subcommand = Some(val.string()?),
            Value(val) if file.is_none() => file = Some(PathBuf::from(val)),
            _ => return Err(arg.unexpected().into()),
        }
    }

    match subcommand.as_deref() {
        None | Some("repl") => {
            if let Some(extra) = file {
                return Err(format!(
                    "Unexpected argument '{}'. Usage: rhesus repl",
                    extra.display()
                )
                .into());
            }
            Ok(Command::Repl { engine })
        }
        Some("run") => {
            let file = file.ok_or("Missing file path. Usage: rhesus run <FILE>")?;
            Ok(Command::Run { file, engine })
        }
        Some("eval") => {
            let file = file.ok_or("Missing file path. Usage: rhesus eval <FILE>")?;
            Ok(Command::Run { file, engine: Engine::Eval })
        }
        Some("compile") => {
            let file = file.ok_or("Missing file path. Usage: rhesus compile <FILE>")?;
            Ok(Command::Compile { file })
        }
        Some(unknown) => {
            Err(format!("Unknown subcommand '{unknown}'. Run 'rhesus --help' for usage.").into())
        }
    }
}
