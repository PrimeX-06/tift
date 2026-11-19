# Changelog

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
