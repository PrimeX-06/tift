#![forbid(unsafe_code)]

#[cfg(windows)]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    std::process::exit(tift_cli::run());
}

#[path = "cli.rs"]
mod tift_cli;
