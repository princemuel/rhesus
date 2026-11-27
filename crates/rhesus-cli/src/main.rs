#![feature(const_cmp)]
#![feature(const_trait_impl)]

mod args;
mod repl;
mod user;

use std::io::Write as _;
use std::process::ExitCode;
use std::{io, panic, thread};

use rhesus_core::evaluator::DEFAULT_STACK_BUDGET;

pub type Result<T = ()> = core::result::Result<T, Box<dyn core::error::Error + Send + Sync>>;

const INTERPRETER_STACK_SIZE: usize = 256 * 1024 * 1024;

fn main() -> ExitCode {
    let outcome = thread::scope(|s| {
        let spawned = thread::Builder::new()
            .name("rhesus".into())
            .stack_size(INTERPRETER_STACK_SIZE)
            .spawn_scoped(s, || args::run(INTERPRETER_STACK_SIZE / 2));

        if let Ok(handle) = spawned {
            handle.join().unwrap_or_else(|p| panic::resume_unwind(p))
        } else {
            args::run(DEFAULT_STACK_BUDGET)
        }
    });

    if let Err(err) = outcome {
        drop(writeln!(io::stderr(), "{err}"));
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
