# Getting started

## Requirements

A platform archive from [GitHub Releases](https://github.com/PrimeX-06/tift/releases) contains a ready-to-run executable. Cargo installation and source builds use Rust 1.99.1 or later with native build tools. On Windows, install the Visual Studio C++ Build Tools and Windows SDK. On macOS, install Xcode Command Line Tools. On Linux, install your distribution's C/C++ compiler and linker packages. TeX and Perl are optional for formatting source; a TeX distribution is useful for checking rendered documents.

## Installation

```sh
cargo install tift --locked --registry crates-io
tift --version
```

Cargo installs the latest published version. Check the installed version with `tift --version`. Installed commands usually live in `~/.cargo/bin` or `%USERPROFILE%\.cargo\bin`; ensure this directory is on `PATH`.

For a platform binary, choose the Windows x86_64 ZIP, Linux x86_64 tar archive, or the matching macOS Apple Silicon/Intel tar archive. Extract it and add the executable's directory to `PATH`. `SHA256SUMS` lists the archive checksums.

To install from the release source:

```sh
cargo +1.99.1 install --git https://github.com/PrimeX-06/tift.git --tag v0.1.4 --locked tift
```

For a local checkout:

```sh
git clone --branch v0.1.4 https://github.com/PrimeX-06/tift.git
cd tift
cargo install --path . --locked
```

The checkout selects Rust 1.99.1 through `rust-toolchain.toml`, including rustfmt and clippy. Rustup downloads missing components. To build a local executable:

```sh
cargo build --release --locked
./target/release/tift --help
```

Windows uses `target\release\tift.exe`.

## First format

Preview, apply, then check:

```sh
tift --diff main.tex
tift --backup main.tex
tift --check main.tex
```

`--diff` returns 1 when changes are found. `--backup` keeps the original as `main.tex.bak`. A successful check returns 0.

To format a project recursively, run `tift .` or `tift chapters/`. Directory scans respect ignore files and configured exclusions. Explicit filenames retain configured exclusions and bypass directory ignore rules. Symlinks are skipped during scans and rejected as explicit inputs.

## Project configuration

Create `tift.toml` beside the main source:

```toml
[indent]
width = 4

[wrap]
mode = "preserve"

[files]
exclude = ["generated/", "vendor/"]
```

Inspect effective settings with `tift --show-config main.tex`. Defaults preserve prose lines and use an 80-column budget for displayed formulas. To wrap long prose lines, use `--wrap --wrap-width 88`; to join old prose lines and wrap the paragraph, use `--rewrap --wrap-width 88`.

For chapters using custom project commands, add `% !TEX root = ../main.tex` to select the root declaration context. File formatting retains the input/include commands.

See [configuration setup](Configuration-Guide) for personal settings, inheritance, and per-file overrides.

## Updating

```sh
cargo install tift --locked --registry crates-io --force
```

For Git or local-checkout installations, select the desired release tag and repeat the installation command with `--force`. Check [releases](https://github.com/PrimeX-06/tift/releases) before updating shared project tooling.

Continue with the [command-line reference](CLI), [configuration reference](Configuration), or [formatting examples](Formatting).
