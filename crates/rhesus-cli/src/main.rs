mod repl;

use std::fs;
use std::io::{BufWriter, Write as _, stdout};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use lexopt::prelude::*;
use rhesus_core::{Engine, Session};

fn main() -> Result<()> {
    match parse_args()? {
        Command::Repl { engine } => {
            repl::start(engine)?;
        }
        Command::Run { file, engine } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("Failed to read file: {}", file.display()))?;

            let mut out = BufWriter::new(stdout().lock());
            let mut session = Session::new(engine);

            let actual = session.eval_line(&source, &mut out);

            match actual {
                Ok(output) => {
                    writeln!(out, "{output}")?;
                    out.flush()?;
                }
                Err(err) => {
                    out.flush()?;
                    bail!("{err}");
                }
            }
        }
        Command::Compile { file } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("Failed to read file: {}", file.display()))?;

            let mut out = BufWriter::new(stdout().lock());
            // TODO: Wire up Compiler::compile(&program) and print bytecode
            // instructions
            writeln!(out, "Compiling {} ({} bytes)...", file.display(), source.len())?;
            out.flush()?;
        }
    }

    Ok(())
}

const HELP: &str = "\
Rhesus - The Monkey programming language interpreter & bytecode VM

USAGE:
    rhesus [SUBCOMMAND] [OPTIONS]

SUBCOMMANDS:
    repl                Start the interactive REPL (default if no subcommand is given)
    run <FILE>          Execute a Monkey script using the bytecode VM
    eval <FILE>         Execute a Monkey script using the tree-walking interpreter
    compile <FILE>      Compile a Monkey script and print bytecode disassembly

OPTIONS:
    -e, --engine <MODE> Select engine for REPL ('vm' or 'eval') [default: vm]
    -h, --help          Print help information
    -V, --version       Print version information
";

enum Command {
    Repl { engine: Engine },
    Run { file: PathBuf, engine: Engine },
    Compile { file: PathBuf },
}

fn parse_args() -> Result<Command> {
    use std::process;

    let mut parser = lexopt::Parser::from_env();
    let mut subcommand: Option<String> = None;
    let mut file: Option<PathBuf> = None;
    let mut engine = Engine::Vm;

    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                let mut out = stdout().lock();
                write!(out, "{HELP}")?;
                out.flush()?;
                process::exit(0);
            }
            Short('V') | Long("version") => {
                let mut out = stdout().lock();
                writeln!(out, "rhesus {}", env!("CARGO_PKG_VERSION"))?;
                out.flush()?;
                process::exit(0);
            }
            Short('e') | Long("engine") => {
                let val: String = parser.value()?.parse()?;

                engine = match val.as_str() {
                    "vm" => Engine::Vm,
                    "eval" => Engine::Eval,
                    other => bail!("Invalid engine '{other}'. Expected 'vm' or 'eval'."),
                };
            }
            Value(val) if subcommand.is_none() => {
                subcommand = Some(val.string()?);
            }
            Value(val) if file.is_none() => {
                file = Some(PathBuf::from(val));
            }
            _ => return Err(arg.unexpected().into()),
        }
    }

    match subcommand.as_deref() {
        None | Some("repl") => Ok(Command::Repl { engine }),
        Some("run") => {
            let file = file.context("Missing file path. Usage: rhesus run <FILE>")?;
            Ok(Command::Run { file, engine })
        }
        Some("eval") => {
            let file = file.context("Missing file path. Usage: rhesus eval <FILE>")?;
            Ok(Command::Run { file, engine: Engine::Eval })
        }
        Some("compile") => {
            let file = file.context("Missing file path. Usage: rhesus compile <FILE>")?;
            Ok(Command::Compile { file })
        }
        Some(unknown) => bail!("Unknown subcommand '{unknown}'. Run 'rhesus --help' for usage."),
    }
}
