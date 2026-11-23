# tift documentation

tift is a configurable formatter for LaTeX and BibTeX source, available as a command-line program and a Rust library. It formats `.tex`, `.tikz`, `.sty`, `.cls`, `.bib`, `.dtx`, and `.ins` files on Windows, Linux, and macOS.

Supported CJK syntax includes ctex, xeCJK, xeCJKfntef, xpinyin, zhnumber, zhlipsum, CJK, CJKfntef, CJKnumb, and luatexja-fontspec. Font options, pronunciation data, Chinese numerals, CJK environments, and underline annotations remain intact while surrounding source is formatted.

Install [tift](Getting-Started), then preview a document with `tift --diff main.tex`. Defaults use four-space indentation, recognized table column specifications, formula whitespace formatting, bibliography layout, and algorithm2e/expl3 layout. Prose line breaks are preserved; bibliography sorting and data cleanup are disabled. Local command definitions and project imports supply environment aliases and signatures.

## Guides

| Page | Contents |
|---|---|
| [Getting started](Getting-Started) | Requirements, installation, first format, and project setup |
| [Command line](CLI) | All options, input modes, exit codes, backups, and safe writes |
| [Configuration setup](Configuration-Guide) | File locations, TOML syntax, inheritance, and per-file overrides |
| [Configuration](Configuration) | Complete TOML reference, defaults, inheritance, and overrides |
| [Formatting](Formatting) | Indentation, alignment, wrapping, arguments, and BibTeX examples |
| [Formatting examples](Formatting-Examples) | Input/output gallery with common formats, custom syntax, replacements, and line selections |
| [BibTeX and BibLaTeX](BibTeX) | Normalization, field ordering, and configurable entry sorting |
| [Formula formatting](Math-Formatting) | Operator spacing, scripts, and displayed formula breaks |
| [Table column alignment](Table-Alignment) | Column specifications, tabularray rules, and spans |
| [Code and docstrip formatting](Code-Formatting) | algorithm2e, expl3, documented TeX sources, and installation scripts |
| [Project context](Project-Context) | Roots, local imports, declaration inference, and custom syntax |
| [Source analysis](Source-Analysis) | Lossless snapshots, aliases, shared rules and conservative diagnostics |
| [Supported syntax](Supported-Syntax) | Package families, literal protection, and analysis limits |
| [Editor integration](Editor-Integration) | LaTeX Workshop, Neovim, pre-commit, and LaTeX-project CI |
| [VS Code / LaTeX Workshop](VS-Code) | Setup steps, copyable settings, formatting on save, and troubleshooting |
| [Library API](Library-API) | Rust integration, results, diagnostics, ranges, and traces |
| [Cache and tracing](Cache-and-Tracing) | Storage, invalidation, line selections, and diagnosis |
| [Project processing](Performance) | Dependency preparation, worker threads, and repeated runs |
| [Building from source](Development) | Local builds, installation, and API documentation |
| [Troubleshooting](FAQ) | Configuration, unusual syntax, output, and installation issues |

## Useful commands

```sh
tift --check .
tift --diff main.tex
tift --show-config main.tex
tift --dump-config > tift.toml
tift --wrap --wrap-width 88 main.tex
tift --rewrap --wrap-width 88 main.tex
tift - --stdin-filename main.tex
```

[Source repository](https://github.com/PrimeX-06/tift) · [Issues](https://github.com/PrimeX-06/tift/issues) · [Releases](https://github.com/PrimeX-06/tift/releases)
