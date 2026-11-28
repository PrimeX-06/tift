# <img src="https://raw.githubusercontent.com/PrimeX-06/tift/main/site/branding/tift-logo.svg" alt="" width="40" height="40"> tift — A fast LaTeX and BibTeX formatter

[![Rust 1.99.1+](https://img.shields.io/badge/Rust-1.99.1%2B-orange)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

tift keeps the source of a shared paper easy to review. Equations and table columns line up, nested lists show their structure, and bibliography entries follow the same layout. Coauthors can share one `tift.toml` and use it in their editors and in CI.

**[Try the Playground](https://primex-06.github.io/tift/#playground)** — Paste your own LaTeX or BibTeX snippet and see the formatted source as you type.

[Install](#install) · [Quick start](#quick-start) · [Editor](#editor) · [User guide](https://github.com/PrimeX-06/tift/wiki)

![LaTeX before and after formatting, showing environment indentation, math spacing, and alignment of equations and table columns](site/branding/tift-latex-formatting.png)

![BibTeX before and after formatting, showing field indentation and aligned equals signs](site/branding/tift-bibtex-formatting.png)

## Install

Download the archive for Windows, Linux, or macOS, extract the executable, and add its directory to `PATH`. [Download tift 0.2.2](https://github.com/PrimeX-06/tift/releases/tag/v0.2.2)

With Rust 1.99.1 or later, you can also install tift using Cargo.

```sh
cargo install tift --locked --registry crates-io
tift --version
```

See the [installation guide](https://github.com/PrimeX-06/tift/wiki/Getting-Started) for platform setup, updates, and troubleshooting.

## Quick start

Preview the diff for one file. When you're happy with it, apply the changes.

```sh
tift --diff main.tex
tift main.tex
```

Working on an existing paper? Run `tift --diff --changed-since HEAD` to preview formatting on just the lines you edited. To format the whole project, run `tift .` from its root. Use `tift --check .` in CI.

You can also pass a bibliography (`references.bib`) or a BibTeX style file (`journal.bst`) in place of `main.tex`. [Command-line guide](https://github.com/PrimeX-06/tift/wiki/CLI)

## Performance

tift 0.2.2 took **27 ms** to check the *Web Scraping with R* slides, **185 ms** for the single-file *Think Python* book, and **566 ms** for ten files from *Roots of Language*. These are single-threaded medians on Windows 11, including CLI startup. All 14 workloads completed (49 input files from 13 repositories).

<details>
<summary>All 14 results and how to reproduce them</summary>

| LaTeX project | Input files | Median (ms) | Range (ms) |
| --- | ---: | ---: | ---: |
| [emcee](https://github.com/dfm/emcee/tree/ff3dc94006167397e4ee2814fea9cb9532caeecc/document) | 1 | 57 | 47–67 |
| [Lomb–Scargle](https://github.com/jakevdp/PracticalLombScargle/tree/5f4ca27deefa7d4edc68da3c92d34f484c5bc98e) | 1 | 67 | 66–72 |
| [Tidy Data](https://github.com/hadley/tidy-data/tree/5623c7a302977e8c88f623a08556d5c08e940d8c) | 1 | 144 | 139–155 |
| [Think Python](https://github.com/AllenDowney/ThinkPython2/tree/cf3d99c002933e1829e4aa89109d2e1dae809de0/book) | 1 | 185 | 183–367 |
| [lshort-zh-cn](https://github.com/CTeX-org/lshort-zh-cn/tree/3b6fbb2f88f81eb12a405db0988e53702cac10ee/src) | 17 | 181 | 171–368 |
| [Roots of Language](https://github.com/langsci/91/tree/bca083ed2f5f30dd4c2d6587366e8e0f649285c3) | 10 | 566 | 554–665 |
| [Lung tomography thesis](https://github.com/habi/PhD-Thesis/tree/2453f652da5d90887e494d01727e8c3cc474f567) | 10 | 364 | 354–392 |
| [Web Scraping with R](https://github.com/eduardszoecs/talk_webscrapingr/tree/61587ea5dca920edf0d71ddfc12c9c4bda77b53c) | 1 | 27 | 26–47 |
| [TeXConf 2022 slides](https://github.com/t-tk/upmendex-package/tree/6850a15281169e1976a2e9dd20fea7fe331052c9/doc/present/src) | 1 | 41 | 40–52 |
| [Résumé](https://github.com/billryan/resume/tree/eadc3955cf006254dc8f2419e971fbe3f5bab413) | 1 | 48 | 46–94 |
| [Statistical cookbook](https://github.com/mavam/stat-cookbook/tree/09bd804a5251911088d55f8ce787a991b8b4ba15) | 2 | 172 | 162–184 |

| BibTeX bibliography | Input files | Median (ms) | Range (ms) |
| --- | ---: | ---: | ---: |
| [Lomb–Scargle references](https://github.com/jakevdp/PracticalLombScargle/blob/5f4ca27deefa7d4edc68da3c92d34f484c5bc98e/PracticalLombScargle.bib) | 1 | 16 | 14–27 |
| [Grammatical Theory references](https://github.com/langsci/25/blob/59eec1d2f610ef92a57aee8dddba66df82047677/gt.bib) | 1 | 24 | 19–34 |
| [WALS Online](https://github.com/cldf-datasets/wals/blob/81298aa26d01fdc11b769bcc7fa87ad9e43d7ccd/cldf/sources.bib) | 1 | 36 | 32–44 |

Each row is one `tift --check` invocation in the original project directory, with one worker and tift's configuration files and cache disabled. After one warm-up per workload, the [benchmark runner](performance/authored_works.py) ran seven measured passes in varying order on a Core i9-14900HX. The range is the fastest to slowest of those seven checks. Downloading and unpacking the pinned sources are outside the timer.

```sh
python performance/authored_works.py --mode formal --tift path/to/tift
```

</details>

## Configuration

tift preserves prose line breaks by default. To wrap at 88 columns and indent by four spaces, save this as `tift.toml` beside your project's `main.tex`. Wrapping also supports Chinese, Japanese, and Korean text.

```toml
[indent]
width = 4

[wrap]
mode = "wrap"
columns = 88
```

tift loads the file automatically when you format the project. [Annotated configuration for 0.2.2](https://github.com/PrimeX-06/tift/blob/v0.2.2/examples/tift.toml)

## Editor

To use tift as the LaTeX formatter in VS Code, install [LaTeX Workshop](https://marketplace.visualstudio.com/items?itemName=James-Yu.latex-workshop) and make sure `tift` is on your `PATH`. Add these settings to `.vscode/settings.json` so the extension runs tift through its `latexindent` backend.

```json
{
    "latex-workshop.formatting.latex": "latexindent",
    "latex-workshop.formatting.latexindent.path": "tift",
    "latex-workshop.formatting.latexindent.args": ["--print", "%TMPFILE%"],
    "[latex]": {
        "editor.defaultFormatter": "James-Yu.latex-workshop"
    }
}
```

Open a `.tex` file and run **Format Document** from the Command Palette. To also format on save, add `"editor.formatOnSave": true` inside the `[latex]` block. [Editor guide](https://github.com/PrimeX-06/tift/wiki/VS-Code)

## Documentation

| Guide | Contents |
|---|---|
| [Formatting examples](https://github.com/PrimeX-06/tift/wiki/Formatting-Examples) | See how documents, tables, equations, and bibliographies are formatted |
| [Math](https://github.com/PrimeX-06/tift/wiki/Math-Formatting), [tables](https://github.com/PrimeX-06/tift/wiki/Table-Alignment), [bibliographies](https://github.com/PrimeX-06/tift/wiki/BibTeX), and [code](https://github.com/PrimeX-06/tift/wiki/Code-Formatting) | Feature behavior and settings |
| [Configuration](https://github.com/PrimeX-06/tift/wiki/Configuration) | Customize formatting rules |
| [Editor integration](https://github.com/PrimeX-06/tift/wiki/Editor-Integration) | VS Code, Neovim, pre-commit, and CI |
| [Supported syntax](https://github.com/PrimeX-06/tift/wiki/Supported-Syntax) | File types, packages, and formatting controls |

[Ask a question](https://github.com/PrimeX-06/tift/discussions) · [Report an issue](https://github.com/PrimeX-06/tift/issues)

## License

[MIT](LICENSE).
