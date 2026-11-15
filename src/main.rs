#![forbid(unsafe_code)]

fn main() {
    std::process::exit(tift_cli::run());
}

#[path = "cli.rs"]
mod tift_cli;
