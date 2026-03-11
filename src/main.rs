mod commands;
mod config;
mod deps;
mod format;
mod index;
mod parse;
mod storage;

fn main() {
    if let Err(error) = commands::run() {
        eprint!("{}", format::render_error(&format!("{error:#}")));
        std::process::exit(2);
    }
}
