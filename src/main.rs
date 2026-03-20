mod annotations;
mod batman;
mod commands;
mod config;
mod deps;
mod diagnostics;
mod format;
mod git_staleness;
mod index;
mod links;
mod parse;
mod search;
mod storage;

fn main() {
    if let Err(error) = commands::run() {
        eprint!("{}", format::render_error(&format!("{error:#}")));
        std::process::exit(2);
    }
}
