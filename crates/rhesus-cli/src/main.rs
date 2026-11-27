#![feature(const_cmp)]
#![feature(const_trait_impl)]

mod args;
mod repl;

use std::io::Write as _;
use std::process::ExitCode;
use std::{io, panic, thread};

use rhesus_core::evaluator::DEFAULT_STACK_BUDGET;

pub type Result<T = ()> = core::result::Result<T, Box<dyn core::error::Error + Send + Sync>>;

const DEFAULT_THREAD_STACK_SIZE: usize = 16 * 1024 * 1024; // 16 MiB (8 MiB budget)
const MIN_THREAD_STACK_SIZE: usize = 2 * 1024 * 1024; // 2 MiB floor

fn main() -> ExitCode {
    let command = match args::parse_args() {
        Ok(cmd) => cmd,
        Err(err) => {
            drop(writeln!(io::stderr(), "{err}"));
            return ExitCode::FAILURE;
        }
    };

    let stack_size =
        command.stack_size().unwrap_or(DEFAULT_THREAD_STACK_SIZE).max(MIN_THREAD_STACK_SIZE);

    let outcome = thread::scope(|s| {
        if let Ok(handle) = thread::Builder::new()
            .name("rhesus".into())
            .stack_size(stack_size)
            .spawn_scoped(s, || args::execute(&command, stack_size / 2))
        {
            handle.join().unwrap_or_else(|p| panic::resume_unwind(p))
        } else {
            args::execute(&command, DEFAULT_STACK_BUDGET)
        }
    });

    if let Err(err) = outcome {
        drop(writeln!(io::stderr(), "{err}"));
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
