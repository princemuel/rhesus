use std::borrow::Cow;
use std::env;
use std::io::{self, BufWriter, Write};

use rhesus_core::errors::Error;
use rhesus_core::host::WriterHost;
use rhesus_core::parser::is_incomplete;
use rhesus_core::{Engine, Session};
use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Editor, Helper};

pub(crate) fn start(engine: Engine, stack_budget: usize) -> crate::Result<()> {
    let mut rl = MonkeyEditor::new()?;
    rl.set_helper(Some(MonkeyHelper));

    // `$HOME/.rhesus_history`, or `None` if there is no home directory.
    let history = env::home_dir().map(|home| home.join(".rhesus_history"));

    // a missing history file on the first run is expected.
    if let Some(path) = &history {
        let _maybe_missing = rl.load_history(path);
    }

    let result = run(&mut rl, engine, stack_budget);

    // Runs on every exit path, including errors from `run`.
    if let Some(path) = &history
        && let Err(err) = rl.save_history(path)
    {
        eprintln!("Could not save history to {}: {err}", path.display());
    }

    result
}

struct MonkeyHelper;
impl Completer for MonkeyHelper {
    type Candidate = String;
}
impl Helper for MonkeyHelper {}
impl Highlighter for MonkeyHelper {}
impl Hinter for MonkeyHelper {
    type Hint = String;
}

impl Validator for MonkeyHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(judge(ctx.input()))
    }
}

type MonkeyEditor = Editor<MonkeyHelper, DefaultHistory>;

fn judge(input: &str) -> ValidationResult {
    if is_incomplete(input) { ValidationResult::Incomplete } else { ValidationResult::Valid(None) }
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

fn run(rl: &mut MonkeyEditor, engine: Engine, stack_budget: usize) -> crate::Result<()> {
    let mut session = Session::new(engine).with_stack_budget(stack_budget);
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
            Err(err) => return Err(format!("REPL error: {err}").into()),
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

const MONKEY_FACE: &str = r#"            __,__
   .--.  .-"     "-.  .--.
  / .. \/  .-. .-.  \/ .. \
 | |  '|  /   Y   \  |'  | |
 | \   \  \ 0 | 0 /  /   / |
  \ '- ,\.-"""""""-./, -' /
   ''-' /_   ^ ^   _\ '-''
       |  \._   _./  |
       \   \ '~' /   /
        '._ '-=-' _.'
           '-----'
"#;

fn eval(
    session: &mut Session,
    src: &str,
    out: &mut impl Write,
    err_out: &mut impl Write,
) -> crate::Result<()> {
    let result = session.eval_line(src, &mut WriterHost(&mut *out));

    match result {
        Ok(Some(value)) => writeln!(out, "{value}")?,
        Ok(None) => {}
        Err(err) => {
            out.flush()?; // flush program output first, before writing the error
            report(&err, err_out)?;
        }
    }
    Ok(out.flush()?)
}

fn report(err: &Error, err_out: &mut impl Write) -> io::Result<()> {
    match err {
        Error::Parse(messages) => {
            writeln!(err_out, "{MONKEY_FACE}")?;
            writeln!(err_out, "Woops! We ran into some monkey business here!")?;
            writeln!(err_out, "  parser errors:")?;

            for message in messages {
                writeln!(err_out, "\t{message}")?;
            }
            Ok(())
        }
        Error::Eval(message) => writeln!(err_out, "ERROR: {message}"),
    }
}

fn banner(out: &mut impl Write, engine: Engine) -> crate::Result<()> {
    let mode = match engine {
        Engine::Eval => "interpreter",
        Engine::Vm => "bytecode VM",
    };

    let name = ["USER", "USERNAME"]
        .into_iter()
        .find_map(|k| env::var(k).ok().filter(|n| !n.is_empty()))
        .map_or(Cow::Borrowed("there"), |n| Cow::Owned(n.to_lowercase()));

    writeln!(out, "Rhesus {} (in {mode} mode) on {}", env!("CARGO_PKG_VERSION"), env::consts::OS)?;
    writeln!(out, "Hello {name}! Type \"quit\" or press Ctrl+D to exit.")?;
    Ok(out.flush()?)
}

#[cfg(test)]
mod tests {
    use rhesus_core::errors::Error;
    use rustyline::validate::ValidationResult;

    use super::{Action, Input, State, judge, report};

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

    #[test]
    fn enter_continues_the_line_while_a_block_is_open() {
        for src in ["if (true) {", "let f = fn(x) {\n  x +", "[1,", "\"abc"] {
            assert!(matches!(judge(src), ValidationResult::Incomplete), "{src:?}");
        }
    }

    #[test]
    fn enter_submits_finished_and_hopeless_input() {
        for src in ["1 + 1", "if (true) { 1 }", "let = 5", ".exit", "quit", "}"] {
            assert!(matches!(judge(src), ValidationResult::Valid(None)), "{src:?}");
        }
    }

    #[test]
    fn syntax_errors_get_the_monkey_and_runtime_errors_do_not() {
        let mut out = Vec::new();
        report(&Error::Parse(vec!["expected x".into(), "expected y".into()]), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("monkey business"));
        assert!(text.contains("parser errors:\n\texpected x\n\texpected y\n"));

        let mut out = Vec::new();
        report(&Error::Eval("identifier not found: x".into()), &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "ERROR: identifier not found: x\n");
    }
}
