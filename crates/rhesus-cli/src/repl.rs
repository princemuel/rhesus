use std::env;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Result, bail};
use rhesus_core::{Engine, Session};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use crate::user::username;

pub(crate) fn start(engine: Engine) -> Result<()> {
    let mut rl = DefaultEditor::new()?;
    let history = history_path();

    // a missing history file on the first run is expected.
    if let Some(path) = &history {
        drop(rl.load_history(path));
    }

    let result = run(&mut rl, engine);

    // Runs on every exit path, including errors from `run`.
    if let Some(path) = &history
        && let Err(err) = rl.save_history(path)
    {
        eprintln!("Could not save history to {}: {err}", path.display());
    }

    result
}

const PROMPT: &str = "🐵 >> ";
const EXIT_HINT: &str = "(To exit, press Ctrl+C again or Ctrl+D or type .exit)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input<'a> {
    Line(&'a str),
    Interrupt,
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    ExitArmed,
}

impl State {
    const fn on(self, input: Input<'_>) -> (Self, Action<'_>) {
        match (self, input) {
            (_, Input::Eof | Input::Line(".exit" | "quit"))
            | (Self::ExitArmed, Input::Interrupt) => (self, Action::Quit),
            (Self::Idle, Input::Interrupt) => (Self::ExitArmed, Action::ShowHint),
            // Any real input disarms the exit.
            (_, Input::Line("")) => (Self::Idle, Action::Nothing),
            (_, Input::Line(src)) => (Self::Idle, Action::Eval(src)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action<'a> {
    Nothing,
    ShowHint,
    Eval(&'a str),
    Quit,
}

fn run(rl: &mut DefaultEditor, engine: Engine) -> Result<()> {
    let mut session = Session::new(engine);
    let mut out = BufWriter::new(io::stdout().lock());
    let mut err_out = io::stderr().lock();

    banner(&mut out, engine)?;

    let mut state = State::Idle;
    loop {
        let read = rl.readline(PROMPT);
        let input = match &read {
            Ok(line) => Input::Line(line.trim()),
            Err(ReadlineError::Interrupted) => Input::Interrupt,
            Err(ReadlineError::Eof) => Input::Eof,
            Err(err) => bail!("REPL error: {err}"),
        };

        let (next, action) = state.on(input);
        state = next;

        match action {
            Action::Nothing => {}
            Action::ShowHint => {
                writeln!(out, "{EXIT_HINT}")?;
                out.flush()?;
            }
            Action::Eval(src) => {
                drop(rl.add_history_entry(src));
                eval(&mut session, src, &mut out, &mut err_out)?;
            }
            Action::Quit => break,
        }
    }

    Ok(out.flush()?)
}

fn eval(
    session: &mut Session,
    src: &str,
    out: &mut impl Write,
    err_out: &mut impl Write,
) -> Result<()> {
    match session.eval_line(src, out) {
        Ok(result) => writeln!(out, "{result}")?,
        Err(err) => {
            out.flush()?; // program output first, then the error
            writeln!(err_out, "Woops! Monkey business:\n  {err}")?;
        }
    }
    Ok(out.flush()?)
}

fn banner(out: &mut impl Write, engine: Engine) -> Result<()> {
    let mode = match engine {
        Engine::Eval => "interpreter",
        Engine::Vm => "bytecode VM",
    };
    let name = username().unwrap_or_else(|| "there".to_owned());

    writeln!(out, "Rhesus {} ({mode}) on {}", env!("CARGO_PKG_VERSION"), env::consts::OS)?;
    writeln!(out, "Hello {name}! Type \"quit\" or press Ctrl+D to exit.")?;
    Ok(out.flush()?)
}

/// `~/.rhesus_history`, or `None` if there is no home directory.
fn history_path() -> Option<PathBuf> { env::home_dir().map(|home| home.join(".rhesus_history")) }

#[cfg(test)]
mod tests {
    use super::{Action, Input, State};

    #[test]
    fn scenario_first_ctrl_c_arms_second_quits() {
        let (s, a) = State::Idle.on(Input::Interrupt);
        assert_eq!((s, a), (State::ExitArmed, Action::ShowHint));
        assert_eq!(s.on(Input::Interrupt).1, Action::Quit);
    }

    #[test]
    fn scenario_typing_disarms() {
        let (s, _) = State::ExitArmed.on(Input::Line("1 + 1"));
        assert_eq!(s, State::Idle);
        let (s, _) = State::ExitArmed.on(Input::Line(""));
        assert_eq!(s, State::Idle);
    }

    #[test]
    fn scenario_exit_commands_and_eof_quit_from_any_state() {
        for state in [State::Idle, State::ExitArmed] {
            assert_eq!(state.on(Input::Line(".exit")).1, Action::Quit);
            assert_eq!(state.on(Input::Line("quit")).1, Action::Quit);
            assert_eq!(state.on(Input::Eof).1, Action::Quit);
        }
    }
}
