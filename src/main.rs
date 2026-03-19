mod annotations;
#[allow(dead_code)]
mod batman;
mod commands;
mod config;
mod deps;
mod diagnostics;
mod format;
mod git_staleness;
mod index;
mod parse;
#[allow(dead_code)]
mod search;
#[allow(dead_code)]
mod storage;

fn main() {
    if let Err(error) = commands::run() {
        eprint!("{}", format::render_error(&format!("{error:#}")));
        std::process::exit(2);
    }
}
