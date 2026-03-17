mod annotations;
#[allow(dead_code)]
mod batman;
#[allow(dead_code)]
mod chunk;
mod commands;
mod config;
mod deps;
#[allow(dead_code)]
mod embeddings;
mod format;
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
