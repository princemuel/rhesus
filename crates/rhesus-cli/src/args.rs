use std::ffi::{OsStr, OsString};
use std::io::{BufWriter, Write as _, stdout};
use std::path::{Path, PathBuf};
use std::{env, fs};

use rhesus_core::host::WriterHost;
use rhesus_core::{Engine, Session};

use crate::{MIN_THREAD_STACK_SIZE, Result, repl};

pub(crate) fn parse_args() -> Result<Command> { parse_args_from(env::args_os().skip(1)) }

pub(crate) fn execute(command: &Command, stack_budget: usize) -> Result<()> {
    match command.clone() {
        Command::Help(text) => write!(stdout().lock(), "{text}")?,
        Command::Version => writeln!(stdout().lock(), "rhesus {}", env!("CARGO_PKG_VERSION"))?,
        Command::Repl { engine, .. } => repl::start(engine, stack_budget)?,
        Command::Run { file, engine, .. } => {
            let source = read_source(&file)?;

            let mut out = BufWriter::new(stdout().lock());
            let mut session = Session::new(engine).with_stack_budget(stack_budget);

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
        Command::Compile { file, ast } => {
            let source = read_source(&file)?;

            let mut out = BufWriter::new(stdout().lock());
            if ast {
                // TODO: Parse &source into AST and print it
                writeln!(out, "Printing AST for {} ({} bytes)...", file.display(), source.len())?;
            } else {
                // TODO: Wire up Compiler::compile(&program) and print bytecode
                // instructions
                writeln!(out, "Compiling {} ({} bytes)...", file.display(), source.len())?;
            }

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
    run <FILE>          Execute a Monkey script (with the engine chosen by --engine)
    eval <FILE>         Execute a Monkey script using the tree-walking interpreter
    compile <FILE>      Compile a Monkey script and print bytecode disassembly
    help [SUBCOMMAND]   Print help information for rhesus or a subcommand

OPTIONS:
    -e, --engine <MODE> Engine for repl and run ('eval' or 'vm') [default: eval]
    --stack-size <SIZE> Thread stack size, e.g. 16M, 256M [default: 16M]
    -h, --help          Print help information
    -V, --version       Print version information
";

const HELP_REPL: &str = "\
Start the interactive Monkey REPL

USAGE:
    rhesus repl [OPTIONS]

OPTIONS:
    -e, --engine <MODE>     Execution engine ('eval' or 'vm') [default: eval]
        --stack-size <SIZE> Thread stack size, e.g. 16M, 256M [default: 16M]
    -h, --help              Print help information
";

const HELP_RUN: &str = "\
Execute a Monkey script

USAGE:
    rhesus run [OPTIONS] <FILE>

ARGS:
    <FILE>              Path to the Monkey script to execute

OPTIONS:
    -e, --engine <MODE>     Execution engine ('eval' or 'vm') [default: eval]
        --stack-size <SIZE> Thread stack size, e.g. 16M, 256M [default: 16M]
    -h, --help              Print help information
";

const HELP_EVAL: &str = "\
Execute a Monkey script using the tree-walking interpreter

USAGE:
    rhesus eval <FILE>

ARGS:
    <FILE>              Path to the Monkey script to execute

OPTIONS:
    --stack-size <SIZE> Thread stack size, e.g. 16M, 256M [default: 16M]
    -h, --help          Print help information
";

const HELP_COMPILE: &str = "\
Compile a Monkey script and print bytecode disassembly

USAGE:
    rhesus compile [OPTIONS] <FILE>

ARGS:
    <FILE>              Path to the Monkey script to compile

OPTIONS:
        --ast           Print the parsed AST instead of bytecode disassembly
    -h, --help          Print help information
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Help(&'static str),
    Version,
    Repl { engine: Engine, stack_size: Option<usize> },
    Run { file: PathBuf, engine: Engine, stack_size: Option<usize> },
    Compile { file: PathBuf, ast: bool },
}

impl Command {
    pub(crate) const fn stack_size(&self) -> Option<usize> {
        match self {
            Self::Repl { stack_size, .. } | Self::Run { stack_size, .. } => *stack_size,
            Self::Help(_) | Self::Version | Self::Compile { .. } => None,
        }
    }
}

fn parse_args_from<I>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = OsString>,
{
    resolve(lex(&mut args.into_iter())?)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Args {
    positionals: Vec<OsString>,
    engine: Option<Engine>,
    ast: bool,
    help: bool,
    version: bool,
    stack_size: Option<usize>,
}

#[expect(clippy::too_many_lines)]
/// split argv into flags and positionals
fn lex(args: &mut impl Iterator<Item = OsString>) -> Result<Args> {
    #[cfg(feature = "lexopt")]
    use lexopt::prelude::*;

    let mut parsed = Args::default();

    cfg_select! {
        feature = "lexopt" => {
            let mut parser = lexopt::Parser::from_args(args);

            while let Some(arg) = parser.next()? {
                match arg {
                    Short('h') | Long("help") => parsed.help = true,
                    Long("ast") => parsed.ast = true,
                    Short('e') => parsed.engine = Some(engine_value("-e", &mut parser)?),
                    Long("engine") => parsed.engine = Some(engine_value("--engine", &mut parser)?),
                    Short('V') => {
                        parsed.version = true;
                        return Ok(parsed);
                    }
                    Long("version") => {
                        if parser.optional_value().is_some() {
                            return Err("Option '--version' does not take a value".into());
                        }
                        parsed.version = true;
                        return Ok(parsed);
                    }
                    Long("stack-size") => {
                        let val = parser.value()?;
                        let s = val.to_str().ok_or("Option '--stack-size' must be valid UTF-8")?;
                        parsed.stack_size = Some(parse_stack_size(s)?);
                    }
                    Value(val) => parsed.positionals.push(val),
                    _ => return Err(arg.unexpected().into()),
                }
            }
        }
        _ => {
            let mut options_done = false; // set once a bare `--` is seen

            while let Some(arg) = args.next() {
                // Non-UTF-8, "-", anything after `--`, and non-dashed args are
                // positionals.
                let Some(text) =
                    arg.to_str().filter(|txt| !options_done && txt.starts_with('-') && *txt != "-")
                else {
                    parsed.positionals.push(arg);
                    continue;
                };

                if text == "--" {
                    options_done = true;
                } else if let Some(long) = text.strip_prefix("--") {
                    let (name, attached) = match long.split_once('=') {
                        Some((name, value)) => (name, Some(value)),
                        None => (long, None),
                    };
                    match (name, attached) {
                        ("engine", _) => {
                            parsed.engine = Some(engine_value("--engine", attached, args)?);
                        }
                        ("help", None) => parsed.help = true,
                        ("ast", None) => parsed.ast = true,
                        ("version", None) => {
                            parsed.version = true;
                            return Ok(parsed);
                        }
                        ("help" | "ast" | "version", Some(_)) => {
                            return Err(format!("Option '--{name}' does not take a value").into());
                        }
                        ("stack-size", _) => {
                            let next_arg;
                            let s = match attached {
                                Some(v) => v,
                                None => {
                                    next_arg = args
                                        .next()
                                        .ok_or("Option '--stack-size' requires a value")?;
                                    next_arg
                                        .to_str()
                                        .ok_or("Option '--stack-size' must be valid UTF-8")?
                                }
                            };
                            parsed.stack_size = Some(parse_stack_size(s)?);
                        }
                        _ => return Err(format!("Unknown option '--{name}'").into()),
                    }
                } else {
                    if let Some(short) = text.strip_prefix('-')
                        && !short.is_empty()
                    {
                        let mut flags = short.chars();
                        while let Some(flag) = flags.next() {
                            match flag {
                                'h' => parsed.help = true,
                                'V' => {
                                    parsed.version = true;
                                    return Ok(parsed);
                                }
                                'e' => {
                                    let rest = flags.as_str();
                                    let attached = (!rest.is_empty())
                                        .then(|| rest.strip_prefix('=').unwrap_or(rest));
                                    parsed.engine = Some(engine_value("-e", attached, args)?);
                                    break;
                                }
                                _ => return Err(format!("Unknown option '-{flag}'").into()),
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(parsed)
}

cfg_select! {
    feature = "lexopt" => {
        fn engine_value(flag: &str, parser: &mut lexopt::Parser) -> Result<Engine> {
            use lexopt::ValueExt as _;

            let raw = parser.value()?.string()?;
            raw.parse().map_err(|e| format!("Invalid value '{raw}' for '{flag}': {e}").into())
        }
    }
    _ => {
        fn engine_value(
            flag: &str,
            attached: Option<&str>,
            args: &mut impl Iterator<Item = OsString>,
        ) -> Result<Engine> {
            let raw = if let Some(value) = attached {
                value.to_owned()
            } else {
                let next = args.next().ok_or_else(|| format!("Missing value for '{flag}'"))?;
                into_utf8(next, flag)?
            };

            raw.parse().map_err(|e| format!("Invalid value '{raw}' for '{flag}': {e}").into())
        }
    }
}

fn resolve(parsed: Args) -> Result<Command> {
    let Args { positionals, engine, ast, help, version, stack_size } = parsed;

    if version {
        return Ok(Command::Version);
    }

    let mut positionals = positionals.into_iter();

    let subcommand = positionals.next().map(|s| into_utf8(s, "subcommand")).transpose()?;
    let subcommand = subcommand.as_deref();
    let operand = positionals.next(); // <FILE>, or the topic of `help <TOPIC>`

    if let Some(extra) = positionals.next() {
        return Err(format!("Unexpected argument '{}'", extra.to_string_lossy()).into());
    }

    if help {
        // `help run --help` asks about `run`; `run --help` does too.
        let topic = match subcommand {
            Some("help") => utf8_operand(operand.as_deref())?,
            other => other,
        };

        return Ok(Command::Help(help_text(topic)?));
    }

    match subcommand {
        None | Some("repl") => {
            forbid_ast(ast)?;
            if let Some(extra) = operand {
                return Err(format!(
                    "Unexpected argument '{}'. Usage: rhesus repl",
                    extra.to_string_lossy()
                )
                .into());
            }
            Ok(Command::Repl { engine: engine.unwrap_or_default(), stack_size })
        }
        Some("run") => {
            forbid_ast(ast)?;
            let file = operand.ok_or("Missing file path. Usage: rhesus run <FILE>")?;
            Ok(Command::Run {
                file: PathBuf::from(file),
                engine: engine.unwrap_or_default(),
                stack_size,
            })
        }
        Some("eval") => {
            forbid_engine(engine.is_some(), "eval")?;
            forbid_ast(ast)?;
            let file = operand.ok_or("Missing file path. Usage: rhesus eval <FILE>")?;
            Ok(Command::Run { file: PathBuf::from(file), engine: Engine::Eval, stack_size })
        }
        Some("compile") => {
            forbid_engine(engine.is_some(), "compile")?;
            forbid_stack_size(stack_size.is_some(), "compile")?;
            let file = operand.ok_or("Missing file path. Usage: rhesus compile <FILE>")?;
            Ok(Command::Compile { file: PathBuf::from(file), ast })
        }
        Some("help") => {
            forbid_engine(engine.is_some(), "help")?;
            forbid_ast(ast)?;
            forbid_stack_size(stack_size.is_some(), "help")?;
            Ok(Command::Help(help_text(utf8_operand(operand.as_deref())?)?))
        }
        Some(unknown) => {
            Err(format!("Unknown subcommand '{unknown}'. Run 'rhesus --help' for usage.").into())
        }
    }
}

fn parse_stack_size(raw: &str) -> Result<usize> {
    let trimmed = raw.trim();

    let (num, mult) = if let Some(s) = trimmed.strip_suffix(['k', 'K']) {
        (s, 1024)
    } else if let Some(s) = trimmed.strip_suffix(['m', 'M']) {
        (s, 1024 * 1024)
    } else if let Some(s) = trimmed.strip_suffix(['g', 'G']) {
        (s, 1024 * 1024 * 1024)
    } else {
        (trimmed, 1)
    };

    let num: usize = num.parse().map_err(|err| {
        format!("Invalid stack size '{raw}': expected bytes or suffix K/M/G ({err})")
    })?;

    let bytes = num.checked_mul(mult).ok_or_else(|| format!("Stack size '{raw}' is too large"))?;

    if bytes < MIN_THREAD_STACK_SIZE {
        return Err(format!(
            "Stack size '{raw}' is too small (minimum is {}M)",
            MIN_THREAD_STACK_SIZE / (1024 * 1024)
        )
        .into());
    }

    Ok(bytes)
}

fn forbid_stack_size(has_stack_size: bool, subcommand: &str) -> Result<()> {
    if has_stack_size {
        Err(format!("Option '--stack-size' cannot be used with '{subcommand}'").into())
    } else {
        Ok(())
    }
}

fn forbid_ast(ast: bool) -> Result<()> {
    if ast { Err("Option '--ast' can only be used with 'compile'.".into()) } else { Ok(()) }
}

fn forbid_engine(given: bool, subcommand: &str) -> Result<()> {
    if given {
        Err(format!("Option '--engine' cannot be used with '{subcommand}'.").into())
    } else {
        Ok(())
    }
}

fn help_text(topic: Option<&str>) -> Result<&'static str> {
    match topic {
        None | Some("help") => Ok(HELP),
        Some("repl") => Ok(HELP_REPL),
        Some("run") => Ok(HELP_RUN),
        Some("eval") => Ok(HELP_EVAL),
        Some("compile") => Ok(HELP_COMPILE),
        Some(unknown) => {
            Err(format!("Unknown subcommand '{unknown}'. Run 'rhesus --help' for usage.").into())
        }
    }
}

fn utf8_operand(operand: Option<&OsStr>) -> Result<Option<&str>> {
    operand.map(|os| os.to_str().ok_or_else(|| "Invalid UTF-8 for subcommand".into())).transpose()
}

fn into_utf8(os_string: OsString, ctx: &str) -> Result<String> {
    os_string
        .into_string()
        .map_err(|bad| format!("Invalid UTF-8 for {ctx}: '{}'", bad.to_string_lossy()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command> { parse_args_from(args.iter().map(OsString::from)) }

    #[test]
    fn defaults_to_repl_with_eval_engine() {
        assert_eq!(parse(&[]).unwrap(), Command::Repl { engine: Engine::Eval, stack_size: None });
        assert_eq!(parse(&["repl"]).unwrap(), Command::Repl {
            engine: Engine::Eval,
            stack_size: None
        });
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse(&["-h"]).unwrap(), Command::Help(HELP));
        assert_eq!(parse(&["--help"]).unwrap(), Command::Help(HELP));
        assert_eq!(parse(&["repl", "--help"]).unwrap(), Command::Help(HELP_REPL));
        assert_eq!(parse(&["run", "--help"]).unwrap(), Command::Help(HELP_RUN));
        assert_eq!(parse(&["--help", "run"]).unwrap(), Command::Help(HELP_RUN));
        assert_eq!(parse(&["eval", "-h"]).unwrap(), Command::Help(HELP_EVAL));
        assert_eq!(parse(&["compile", "--help"]).unwrap(), Command::Help(HELP_COMPILE));
        assert!(parse(&["unknown", "--help"]).is_err());
        assert_eq!(parse(&["-V"]).unwrap(), Command::Version);
        assert_eq!(parse(&["--version"]).unwrap(), Command::Version);
    }

    #[test]
    fn help_subcommand() {
        assert_eq!(parse(&["help"]).unwrap(), Command::Help(HELP));
        assert_eq!(parse(&["help", "help"]).unwrap(), Command::Help(HELP));
        assert_eq!(parse(&["help", "repl"]).unwrap(), Command::Help(HELP_REPL));
        assert_eq!(parse(&["help", "run"]).unwrap(), Command::Help(HELP_RUN));
        assert_eq!(parse(&["help", "eval"]).unwrap(), Command::Help(HELP_EVAL));
        assert_eq!(parse(&["help", "compile"]).unwrap(), Command::Help(HELP_COMPILE));
        assert_eq!(parse(&["help", "run", "--help"]).unwrap(), Command::Help(HELP_RUN));
        assert!(parse(&["help", "unknown"]).is_err());
        assert!(parse(&["help", "run", "extra"]).is_err());
        assert!(parse(&["help", "--engine", "vm"]).is_err());
        assert!(parse(&["help", "--ast"]).is_err());
    }

    #[test]
    fn engine_flag_variants() {
        let expected =
            Command::Run { file: PathBuf::from("main.mk"), engine: Engine::Vm, stack_size: None };
        assert_eq!(parse(&["run", "main.mk", "-e", "vm"]).unwrap(), expected);
        assert_eq!(parse(&["run", "main.mk", "-evm"]).unwrap(), expected);
        assert_eq!(parse(&["run", "main.mk", "--engine", "vm"]).unwrap(), expected);
        assert_eq!(parse(&["run", "main.mk", "--engine=vm"]).unwrap(), expected);
        assert_eq!(parse(&["-e", "vm", "run", "main.mk"]).unwrap(), expected);
    }

    #[test]
    fn eval_and_compile_subcommands() {
        assert_eq!(parse(&["eval", "script.mk"]).unwrap(), Command::Run {
            file: PathBuf::from("script.mk"),
            engine: Engine::Eval,
            stack_size: None,
        });
        assert_eq!(parse(&["compile", "script.mk"]).unwrap(), Command::Compile {
            file: PathBuf::from("script.mk"),
            ast: false,
        });
        assert_eq!(parse(&["compile", "script.mk", "--ast"]).unwrap(), Command::Compile {
            file: PathBuf::from("script.mk"),
            ast: true,
        });
        assert_eq!(parse(&["--ast", "compile", "script.mk"]).unwrap(), Command::Compile {
            file: PathBuf::from("script.mk"),
            ast: true,
        });
    }

    #[test]
    fn double_dash_allows_dash_prefixed_filenames() {
        assert_eq!(parse(&["run", "--", "-weird-name.mk"]).unwrap(), Command::Run {
            file: PathBuf::from("-weird-name.mk"),
            engine: Engine::Eval,
            stack_size: None
        });
    }

    #[test]
    fn rejects_disallowed_flag_combinations() {
        assert!(parse(&["eval", "script.mk", "--engine", "vm"]).is_err());
        assert!(parse(&["eval", "script.mk", "-e", "eval"]).is_err());
        assert!(parse(&["compile", "script.mk", "--engine=vm"]).is_err());
        assert!(parse(&["--ast"]).is_err());
        assert!(parse(&["repl", "--ast"]).is_err());
        assert!(parse(&["run", "script.mk", "--ast"]).is_err());
        assert!(parse(&["eval", "script.mk", "--ast"]).is_err());
    }

    #[test]
    fn error_cases() {
        assert!(parse(&["run"]).is_err()); // missing file
        assert!(parse(&["eval"]).is_err()); // missing file
        assert!(parse(&["compile"]).is_err()); // missing file
        assert!(parse(&["repl", "extra.mk"]).is_err()); // unexpected positional
        assert!(parse(&["run", "a.mk", "b.mk"]).is_err()); // extra positional
        assert!(parse(&["unknown"]).is_err()); // bad subcommand
        assert!(parse(&["--unknown"]).is_err()); // bad long flag
        assert!(parse(&["-x"]).is_err()); // bad short flag
        assert!(parse(&["--engine"]).is_err()); // missing flag value
        assert!(parse(&["-e"]).is_err()); // missing short flag value
        assert!(parse(&["--help=true"]).is_err()); // unexpected value on bool flag
        assert!(parse(&["--ast=true"]).is_err()); // unexpected value on bool flag
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_file_paths() {
        use std::os::unix::ffi::OsStringExt as _;

        let bad_utf8 = OsString::from_vec(vec![0xff, 0xfe, b'.', b'm', b'k']);
        let cmd = parse_args_from(vec![OsString::from("run"), bad_utf8.clone()]).unwrap();
        assert_eq!(cmd, Command::Run {
            file: PathBuf::from(bad_utf8),
            engine: Engine::Eval,
            stack_size: None
        });
    }

    #[test]
    fn short_engine_accepts_equals_like_lexopt() {
        let expected =
            Command::Run { file: PathBuf::from("main.mk"), engine: Engine::Vm, stack_size: None };
        assert_eq!(parse(&["run", "main.mk", "-e=vm"]).unwrap(), expected);
        assert!(parse(&["run", "main.mk", "-e="]).is_err());
        assert!(parse(&["run", "main.mk", "--engine="]).is_err());
    }

    #[test]
    fn engine_flag_semantics() {
        assert_eq!(parse(&["repl", "-e", "vm"]).unwrap(), Command::Repl {
            engine: Engine::Vm,
            stack_size: None
        });
        // last one wins
        assert_eq!(parse(&["run", "a.mk", "-e", "eval", "-e", "vm"]).unwrap(), Command::Run {
            file: PathBuf::from("a.mk"),
            engine: Engine::Vm,
            stack_size: None
        });
        let msg = parse(&["run", "a.mk", "-e", "bogus"]).unwrap_err().to_string();
        assert!(msg.contains("bogus") && msg.contains("-e"), "{msg}");
    }

    #[test]
    fn short_flag_clusters() {
        assert_eq!(parse(&["-he", "vm"]).unwrap(), Command::Help(HELP));
        assert_eq!(parse(&["-h", "-V"]).unwrap(), Command::Version);
        assert_eq!(parse(&["-V", "-h"]).unwrap(), Command::Version);
        assert!(parse(&["-hx"]).is_err());
    }

    #[test]
    fn dash_and_double_dash_edge_cases() {
        let file = |f: &str| Command::Run {
            file: PathBuf::from(f),
            engine: Engine::Eval,
            stack_size: None,
        };
        assert_eq!(parse(&["run", "-"]).unwrap(), file("-"));
        assert_eq!(parse(&["run", "--", "--help"]).unwrap(), file("--help"));
        assert_eq!(parse(&["help", "--", "run"]).unwrap(), Command::Help(HELP_RUN));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_subcommand_is_an_error_not_a_panic() {
        use std::os::unix::ffi::OsStringExt as _;

        let bad = OsString::from_vec(vec![0xff, 0xfe]);
        assert!(parse_args_from(vec![bad]).is_err());
    }

    #[test]
    fn stack_size_flag_variants() {
        assert_eq!(parse(&["--stack-size", "32M"]).unwrap(), Command::Repl {
            engine: Engine::Eval,
            stack_size: Some(32 * 1024 * 1024)
        });
        assert_eq!(parse(&["run", "main.mk", "--stack-size=64m"]).unwrap(), Command::Run {
            file: PathBuf::from("main.mk"),
            engine: Engine::Eval,
            stack_size: Some(64 * 1024 * 1024),
        });
        assert_eq!(parse(&["eval", "main.mk", "--stack-size", "2097152"]).unwrap(), Command::Run {
            file: PathBuf::from("main.mk"),
            engine: Engine::Eval,
            stack_size: Some(2 * 1024 * 1024),
        });

        // Rejected on non-executing subcommands and invalid sizes
        assert!(parse(&["compile", "script.mk", "--stack-size", "32M"]).is_err());
        assert!(parse(&["help", "--stack-size", "32M"]).is_err());
        assert!(parse(&["--stack-size", "1M"]).is_err()); // below 2M floor
        assert!(parse(&["--stack-size", "abc"]).is_err());
        assert!(parse(&["--stack-size"]).is_err());
    }
}
