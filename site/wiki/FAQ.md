# Troubleshooting

## The executable is unavailable

Check `cargo install --list` and ensure Cargo's `bin` directory is on `PATH`. On Windows this is normally `%USERPROFILE%\.cargo\bin`; on Unix it is normally `~/.cargo/bin`. A checkout build produces `target/release/tift` or `target\release\tift.exe`.

For linker errors, install the platform's native build tools. To select the minimum compiler explicitly, use `cargo +1.99.1 build --release --locked`.

## A width setting has no effect

`wrap.columns` and `--wrap-width` set the column budget. Enable prose wrapping with `wrap.mode = "wrap"` / `--wrap` to retain old line boundaries, or `wrap.mode = "rewrap"` / `--rewrap` to join old lines before wrapping. Inline math, literals, or unsupported arguments may exceed the target width. Displayed formulas use the width independently while `[math] enabled = true`.

Argument layout needs `arguments.columns`, an active wrap width, or `mode = "expand"`. A preserved named rule overrides automatic expansion.

## The expected configuration is missing

Run `tift --show-config main.tex` to inspect configuration sources and matching overrides. Discovery stops at the nearest `tift.toml`, a Git root, or filesystem root. A nearer configuration takes over as a whole; use `extend` for inheritance. A personal configuration is used only when project discovery finds none.

Paths and patterns are relative to the configuration declaring them. Arrays replace inherited arrays. Command-line options take precedence. Use `--config PATH` for explicit selection or `--no-config` for defaults.

Generate a starting file with `tift --dump-config > tift.toml`. Field names are exact and unknown fields are rejected. Width settings use `wrap.columns` and `arguments.columns`; text-stage settings use `wrap.stage` and `sentences.stage`. The [configuration reference](Configuration) lists the accepted names and values.

## A chapter's custom environment is misread

Add `% !TEX root = ../main.tex` to the chapter, then verify root and local input paths. Supported declarations from preceding local imports contribute context. Dynamic imports, unknown path macros, installed TeX search paths, and complex parameter grammars need explicit signatures or protection. See [Project context](Project-Context).

## A custom table is not aligned

Declare its complete header and select it with `alignment.rules.<name>.enabled = true`. Check the delimiter and row regexes. An explicit environments array replaces the built-in selection. Locally redefined package environments retain conservative custom layout until explicitly selected.

## Formatting controls are ignored

Use exact spaced spellings: `% tift: off`, `% tift: on`, and `% tift: skip`. Compact spellings are ordinary comments. Markers in verbatim code or macro bodies keep their data role. A skipped BibTeX field preserves its whole entry. Whole-source replacements can affect protected regions.

## A line selection leaves some changes unapplied

Selections preserve unselected physical lines. Transformations that redistribute content across a boundary retain original source and produce a warning. Select the entire paragraph or structural block.

## A directory scan misses a file

Check `.gitignore`, `.ignore`, and `files.exclude`. Explicit file arguments bypass directory ignores and retain configured exclusions. Supported extensions are case-insensitive `.tex`, `.tikz`, `.sty`, `.cls`, `.bib`, `.dtx`, and `.ins`. Symlinks are skipped during recursive scans and rejected explicitly.

## A check exits with status 1

Status 1 indicates a formatting difference for `--check`, `--diff`, or `--fail-on-change`. Status 2 indicates arguments, configuration, input, or formatting errors. Use `--diff` to inspect the required changes. `--fail-on-change` can return 1 after successfully writing them.

## Cache storage fails

Formatting falls back to actual processing for corrupt records or unavailable storage. Use `--no-cache` to disable persistent files, or select a writable `--cache-dir`. A source changed by an in-place write needs one actual pass before that result is cached for the new bytes.

## Source is preserved with a warning

Conservative protection applies to uncertain macro, conditional, expl3, and dynamic catcode syntax. Use `--trace-detail --print main.tex` to inspect the stage and retained source. Add a precise signature or raw protection rule for custom grammar. Compile the document when its layout relies on unusual execution-dependent source.

A structural error causes the complete file to retain its original bytes and the CLI to exit with status 2. This is an unsuccessful formatting result. The source may contain an error, or it may use valid syntax beyond the parser's current coverage. Located diagnostics and a minimal reproducer help distinguish those cases. Error recovery alone verifies preservation of data.

## Reporting a bug

Include a minimal input and required declarations, expected output, exact command, `tift --version`, and effective TOML. Keep relevant comments and spacing in the reproducer. Share only the trace excerpts needed to show the issue. File reports at [GitHub Issues](https://github.com/PrimeX-06/tift/issues).
