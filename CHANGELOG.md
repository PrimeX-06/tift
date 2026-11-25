# Changelog

## 0.1.9

- Format classic BibTeX `.bst` styles with indented function bodies, compact field lists, and optional trailing-comment alignment.
- Format supported source types declared in `filecontents` and `filecontents*` using the same settings as standalone files. Set `bibtex.format-filecontents = false` to preserve the embedded source.
- Indent matched algpseudocode branches, loops, functions, and procedures inside `algorithmic`. Set `indent.algpseudocode = false` to preserve the original indentation.
- Keep consecutive same-row booktabs `\cmidrule` commands together by default. Set `breaks.split-cmidrules = true` for one rule per line.
- When wrapping prose, prefer nearby sentence endings and keep reference names and common number–unit pairs together. Enable `wrap.avoid-lone-last-word` to keep a short final pair of English words together. Wrapped comment lists use hanging indents.

[Download tift 0.1.9](https://github.com/PrimeX-06/tift/releases/tag/v0.1.9) or install with `cargo install tift --version 0.1.9 --locked --registry crates-io`.

## 0.1.8

- Align each equation pair in `align`, `alignat`, `aligned`, and related environments with a right-aligned left side and a left-aligned right side. Use the three native column directions in `eqnarray`.

- Align trailing equation labels in supported multi-equation displays by default, within the configured column budget.

- Measure table cells in Unicode display columns and align equals signs in selected multiline command options by default.

- Account for `\multicolumn` spans by default when aligning table source, including tables with uniform justification.
- Finish nonempty, editable files with a newline by default. Set `output.final-newline = "preserve"` to retain the original end of file.
- Configure comment wrapping and alignment with `comments.wrap` and `comments.align-trailing`. Set file endings in `[output]`, the blank-line limit with `whitespace.blank-lines`, and replacement matching with `replacements[].match`.
- Format embedded bibliographies in `filecontents` and `filecontents*` using the same settings as standalone BibTeX files. Set `bibtex.format-filecontents = false` to preserve the embedded source.
- Align literal whitespace-separated PGFPlots tables by default. Set `breaks.pgfplots-tables = false` to preserve inline data or `breaks.pgfplots-named-tables = false` for named tables.
- Add source layout options for bibliography lists and concatenations, repeated command arguments, TikZ paths, and plotting coordinates.

Download a binary from the [GitHub Release](https://github.com/PrimeX-06/tift/releases/tag/v0.1.8) or install with `cargo install tift --version 0.1.8 --locked --registry crates-io`. See the [configuration reference](https://github.com/PrimeX-06/tift/wiki/Configuration) for the current keys.

## 0.1.7

- Collapse repeated spaces in ordinary LaTeX prose while preserving existing line breaks. Set `wrap.collapse-spaces = false` to keep repeated spaces. Comments, verbatim content, and literal command arguments retain their spacing.
- Collapse repeated spaces in ordinary BibTeX field values while preserving expression syntax, string definitions, and literal fields such as URLs and DOIs.
- Try LaTeX and BibTeX formatting in the [browser Playground](https://primex-06.github.io/tift/#playground), with taller source and output panels on desktop and mobile.

Download the archive for Windows, Linux, or macOS from the [GitHub Release](https://github.com/PrimeX-06/tift/releases/tag/v0.1.7), extract the executable, and add its directory to `PATH`. Cargo installation requires Rust 1.99.1 or later.

```sh
cargo install tift --version 0.1.7 --locked --registry crates-io
```

[User guide](https://github.com/PrimeX-06/tift/wiki) · [Configuration](https://github.com/PrimeX-06/tift/wiki/Configuration) · [Editor setup](https://github.com/PrimeX-06/tift/wiki/Editor-Integration)

## 0.1.6

- Expand syntax and formatting coverage for expl3 literal data, hook callbacks, opaque source regions, and deeply nested environments.
- Improve repeated-format performance by reusing activity scans, borrowing expl3 call metadata, reducing opaque-marker allocations, and indexing cross-argument boundaries.
- Add regression coverage and release-mode benchmarks for syntax stability and real-corpus throughput.

Download the archive for Windows, Linux, or macOS from the [GitHub Release](https://github.com/PrimeX-06/tift/releases/tag/v0.1.6), extract the executable, and add its directory to `PATH`. Cargo installation requires Rust 1.99.1 or later.

```sh
cargo install tift --version 0.1.6 --locked --registry crates-io
```

[User guide](https://github.com/PrimeX-06/tift/wiki) · [Configuration](https://github.com/PrimeX-06/tift/wiki/Configuration) · [Editor setup](https://github.com/PrimeX-06/tift/wiki/Editor-Integration)

## 0.1.5

- Improve TeX parsing for scoped expl3 syntax, conditionals, custom environments, and delayed command arguments. Preserve literal and macro-definition content across indentation, alignment, and math formatting.
- Preserve declared source encodings when reading and writing LaTeX and BibTeX files, including encoding declarations in header comments.
- Serialize concurrent writes and backup rotation for the same output path, using stable lock files alongside atomic file replacements.
- Refine BibTeX cleanup and normalization, and expand support for document, reference, key-definition, and code-listing commands.
- Improve project context, configuration handling, and persistent-cache behavior for the expanded syntax and encoding support.

Download the archive for your platform, extract the executable, and add its directory to `PATH`. The archive includes a sample configuration. Cargo installation requires Rust 1.99.1 or later.

```sh
cargo install tift --version 0.1.5 --locked --registry crates-io
```

[User guide](https://github.com/PrimeX-06/tift/wiki) · [Configuration](https://github.com/PrimeX-06/tift/wiki/Configuration) · [Editor setup](https://github.com/PrimeX-06/tift/wiki/Editor-Integration)

## 0.1.4

- Configure horizontal formula spacing separately from display line breaks. Keep existing lines, join eligible display bodies onto one line, or wrap to a separate math width with optional operator priorities.
- Enable optional layout for multiline key/value assignments, table column specifications, literal `\foreach` lists, and `forest` and `qtree` bracket trees. These options preserve the previous layout by default.
- Preserve literal bodies in more code and pseudocode environments, including `algorithmic`, and recognize more custom listing declarations from minted, PythonTeX, codehigh, and tcolorbox.
- Expand support for document and scientific commands, including date and exercise settings, CJK authoring, equation helpers, mathpartir proofs, and physics2 matrices. Literal options and metadata retain their contents.
- Keep the hanging indentation of list prose when a long item title wraps. Reduce repeated parsing during indentation and math formatting.

Download the archive for your platform, extract the executable, and add its directory to `PATH`. The archive includes a sample configuration. Cargo installation requires Rust 1.99.1 or later.

```sh
cargo install tift --version 0.1.4 --locked --registry crates-io
```

[User guide](https://github.com/PrimeX-06/tift/wiki) · [Configuration](https://github.com/PrimeX-06/tift/wiki/Configuration) · [Editor setup](https://github.com/PrimeX-06/tift/wiki/Editor-Integration)

## 0.1.3

- Expanded support for LaTeX packages, CJK writing, custom environments, and locally defined command signatures.
- Added `tift --dump-config` to export a complete starting configuration.
- Added optional cleanup for BibTeX entries, fields, titles, and URLs.

[Earlier releases](https://github.com/PrimeX-06/tift/releases)
