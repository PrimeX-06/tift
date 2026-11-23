# tift — A fast LaTeX and BibTeX formatter

![tift logo](https://raw.githubusercontent.com/PrimeX-06/tift/main/site/branding/tift-logo.svg)

[![Rust 1.99.1+](https://img.shields.io/badge/Rust-1.99.1%2B-orange)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

tift is a fast, configurable formatter for LaTeX and BibTeX. Keep documents consistently indented, align tables and equations, and format bibliographies from the command line or your editor. Available on Windows, Linux, and macOS.

**[Homepage](https://primex-06.github.io/tift/)** · **[User guide](https://github.com/PrimeX-06/tift/wiki)** · **[Formatting examples](https://github.com/PrimeX-06/tift/wiki/Formatting-Examples)** · **[Discussions](https://github.com/PrimeX-06/tift/discussions)** · **[Report an issue](https://github.com/PrimeX-06/tift/issues)**

![LaTeX before and after formatting, showing environment indentation, math spacing, and alignment of equations and table columns](site/branding/tift-latex-formatting.png)

![BibTeX before and after formatting, showing field indentation and aligned equals signs](site/branding/tift-bibtex-formatting.png)

## Features

- Indent LaTeX environments, lists, and nested structures.
- Align table columns, equations, and matrices, and format math spacing.
- Format, sort, and optionally clean up BibTeX and BibLaTeX bibliographies.
- Wrap long prose and captions to a chosen line width.
- Support common LaTeX packages and CJK documents.
- Preserve verbatim content and allow local formatting controls.
- Format whole projects, preview changes, and check formatting in CI.

## Install

### Download a prebuilt binary (recommended)

Download an archive for Windows, Linux, or macOS from the [latest GitHub Release](https://github.com/PrimeX-06/tift/releases/latest), extract the executable, and add its directory to `PATH`.

### Install with Cargo

Install with Cargo using **Rust 1.99.1 or later**:

```sh
cargo install tift --locked --registry crates-io
tift --version
```

See the [installation guide](https://github.com/PrimeX-06/tift/wiki/Getting-Started) for platform setup, updates, and troubleshooting.

## Quick start

```sh
tift --diff main.tex                # preview changes
tift main.tex                       # format a file in place
tift .                              # format a project recursively
tift --check .                      # check formatting in CI
tift --wrap --wrap-width 88 main.tex # wrap long prose
tift --dump-config > tift.toml      # create a project configuration
```

Formatting preserves prose line breaks by default. Add `--backup` to keep a copy of each file before formatting.

Concurrent tift processes serialize writes to the same output path, including backup rotation. Hidden `.tift-lock-*` files next to outputs provide a stable lock across atomic replacements.

Files use UTF-8 by default. Declare another encoding in a header comment, such as `% !TeX encoding = windows-1252` or `% Encoding: latin1` in a bibliography. Formatting preserves the declared encoding.

Use `tift --help` or the [command-line guide](https://github.com/PrimeX-06/tift/wiki/CLI) for more options.

## Configuration

To customize formatting, create **`tift.toml`** beside your project's `main.tex`:

```toml
[indent]
width = 4

[wrap]
mode = "wrap"
columns = 88
```

tift discovers project settings automatically. The [configuration guide](https://github.com/PrimeX-06/tift/wiki/Configuration-Guide) explains setup; the [reference](https://github.com/PrimeX-06/tift/wiki/Configuration) lists all options.

Use `tift --dump-config > tift.toml` to start with all built-in settings, or `tift --show-config main.tex` to inspect a file's effective configuration.

## Editor

To format on save in VS Code, install [LaTeX Workshop](https://marketplace.visualstudio.com/items?itemName=James-Yu.latex-workshop) and add this to `.vscode/settings.json`:

```json
{
    "latex-workshop.formatting.latex": "latexindent",
    "latex-workshop.formatting.latexindent.path": "tift",
    "latex-workshop.formatting.latexindent.args": ["--print", "%TMPFILE%"],
    "[latex]": {
        "editor.defaultFormatter": "James-Yu.latex-workshop",
        "editor.formatOnSave": true
    }
}
```

See the [editor guide](https://github.com/PrimeX-06/tift/wiki/VS-Code) for setup details and troubleshooting.

## Documentation

| Guide | Contents |
|---|---|
| [Formatting examples](https://github.com/PrimeX-06/tift/wiki/Formatting-Examples) | See how documents, tables, equations, and bibliographies are formatted |
| [Math](https://github.com/PrimeX-06/tift/wiki/Math-Formatting), [tables](https://github.com/PrimeX-06/tift/wiki/Table-Alignment), [bibliographies](https://github.com/PrimeX-06/tift/wiki/BibTeX), and [code](https://github.com/PrimeX-06/tift/wiki/Code-Formatting) | Feature behavior and settings |
| [Configuration](https://github.com/PrimeX-06/tift/wiki/Configuration) | Customize formatting rules |
| [Editor integration](https://github.com/PrimeX-06/tift/wiki/Editor-Integration) | VS Code, Neovim, pre-commit, and CI |
| [Supported syntax](https://github.com/PrimeX-06/tift/wiki/Supported-Syntax) | File types, packages, and formatting controls |

## License

[MIT](LICENSE).
