#![forbid(unsafe_code)]

#[cfg(windows)]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(tift_cli::run() as u8)
}

#[path = "cli.rs"]
mod tift_cli;
