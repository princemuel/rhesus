use std::io::{self, BufWriter, Write as _};

use anyhow::Result;
use rhesus_core::{Engine, Session};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

pub(crate) fn start(engine: Engine) -> Result<()> {
    let mut rl = DefaultEditor::new()?;
    let mut session = Session::new(engine);

    let mut out = BufWriter::new(io::stdout().lock());
    let mut err_out = io::stderr().lock();

    let mode = match engine {
        Engine::Eval => "interpreter",
        Engine::Vm => "bytecode VM",
    };

    writeln!(out, "Rhesus {} ({mode} mode)", env!("CARGO_PKG_VERSION"))?;
    writeln!(out, "Type Monkey code and press Enter. Press Ctrl+D to exit.\n")?;
    out.flush()?;

    loop {
        match rl.readline("🐵 >> ") {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let _entered = rl.add_history_entry(trimmed);

                match session.eval_line(trimmed, &mut out) {
                    Ok(result) => {
                        writeln!(out, "{result}")?;
                        out.flush()?;
                    }
                    Err(err) => {
                        out.flush()?;
                        writeln!(err_out, "Woops! Monkey business:\n  {err}")?;
                        err_out.flush()?;
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                writeln!(out, "CTRL-C")?;
                out.flush()?;
                break;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                writeln!(err_out, "REPL Error: {err}")?;
                err_out.flush()?;
                break;
            }
        }
    }

    Ok(())
}
