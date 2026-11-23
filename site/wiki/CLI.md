# Command-line reference

```text
tift [OPTIONS] [PATHS]...
```

Paths can name files and directories. With no paths, the current directory is scanned recursively for `.tex`, `.tikz`, `.sty`, `.cls`, `.bib`, `.dtx`, and `.ins`. Use one `-` input to read stdin. `--stdin-filename` supplies its file kind and path for configuration and overrides. TikZ source fragments use ordinary TeX formatting, including paths, loops, math, and project declarations.

## Input and output modes

| Option | Behavior |
|---|---|
| `--check` | Preserve inputs and return 1 when formatting differs |
| `--diff` | Preserve inputs and print a unified diff; return 1 when it differs |
| `--print`, `-p` | Print one formatted input to stdout |
| `--output PATH`, `-o PATH` | Write one formatted input to a separate file |
| `--backup[=N]` | Keep rotating original copies for changed in-place files |
| `--fail-on-change` | Return 1 when content changes, including after writes |
| `--stdin-filename PATH` | Select stdin type, configuration, and overrides through a virtual path |
| `--lines RANGES`, `-n RANGES` | Format one-based selected lines |

Without a preview or separate-output mode, files are formatted in place. `--check` and `--diff` can be combined. `--print` conflicts with both. `--output` conflicts with preview modes and backups; `--backup` also conflicts with preview modes. Print and output require one input. The destination directory must already exist.

```sh
tift --check .
tift --diff main.tex chapters/
tift --print main.tex
tift - --stdin-filename main.tex < main.tex
tift -o formatted.tex main.tex
tift --backup=3 main.tex
tift --lines '3,7-12,!9' main.tex
```

Range expressions accept single lines, closed ranges, comma-separated selections, and `!` exclusions. Unselected physical lines retain their bytes. Cross-boundary transformations are retained as original source and produce warnings; select the complete paragraph or block when necessary.

## File encodings

Files and stdin use UTF-8 by default. To select another encoding, put an encoding magic comment in the initial comment block, within the first 20 physical lines:

```tex
% !TeX encoding = windows-1252
```

Bibliographies can also use the JabRef-style declaration:

```bibtex
% Encoding: latin1
```

Comment keywords and encoding labels are case insensitive. Common labels include `UTF-8`, `latin1`, `latin2`, `latin9`, `cp1251`, `cp1252`, `GBK`, `GB18030`, `Big5`, and `Shift_JIS`. The TeXworks label `UTF-8 Unicode` is accepted. UTF-16 requires both a BOM and an explicit UTF-16 encoding comment. UTF-8 BOMs are preserved.

Each project input uses its own header. A declaration inside document content has no effect. Commands such as `\usepackage[latin1]{inputenc}` do not select the formatter's file encoding.

In-place writes, separate output, and formatted stdout preserve the input encoding. Unified diffs use UTF-8. Diagnostic byte positions refer to the original encoded file; stage traces describe the decoded UTF-8 text. Backups preserve the original bytes.

Unknown or conflicting declarations, invalid byte sequences, BOM conflicts, and output characters that cannot roundtrip through the original encoding produce an error and preserve the file. Encoding detection does not guess a legacy encoding for an undeclared file. Stateful ISO-2022-JP is unsupported.

## Configuration and layout

| Option | Behavior |
|---|---|
| `--config PATH` | Read this TOML file |
| `--no-config` | Use built-in defaults plus CLI options |
| `--dump-config` | Print an editable default TOML configuration |
| `--show-config` | Print effective settings for one target without formatting it |
| `--indent-width N` | Set spaces per level; 1–256 |
| `--tabs` | Use tab indentation |
| `--wrap` | Enable configured prose wrapping |
| `--rewrap` | Join eligible old prose lines, then wrap them to the configured width |
| `--no-wrap` | Preserve prose line breaks |
| `--wrap-width N` | Set a positive prose/display width; enable prose reflow separately |
| `--modify-line-breaks` | Enable configured structural break rules |
| `--replacement-only` | Run replacement stages without other formatting |

`--config` conflicts with `--no-config`; `--wrap`, `--rewrap`, and `--no-wrap` are mutually exclusive. `--show-config` excludes formatting and trace modes. It accepts a virtual path and reports configuration sources, matched overrides, and exclusions. Root comments and source declarations are resolved during actual formatting.

Generate a project file with `tift --dump-config > tift.toml`. The command prints stable defaults and leaves package-derived environment lists dynamic. Use `--show-config` when you need the effective settings for a particular source path.

`--dump-config` runs on its own, without input paths or other options. It does not read configuration files or format sources. Its TOML output has the same effective values as built-in defaults; optional unset values and automatically maintained lists are omitted.

```sh
tift --rewrap --wrap-width 88 main.tex
tift --print - --stdin-filename package.dtx < package.dtx
```

BibTeX normalization, sorting, column justification, algorithm2e, and expl3 settings are configured in TOML. See [BibTeX and BibLaTeX](BibTeX), [code formatting](Code-Formatting), and the [configuration reference](Configuration).

## Performance and diagnostics

| Option | Behavior |
|---|---|
| `--threads N`, `-j N` | Positive maximum parallel worker count |
| `--no-cache` | Disable persistent result-cache reads and writes |
| `--cache-dir PATH` | Set the cache directory relative to the current working directory |
| `--trace` | Show stage changes, byte counts, and timing on stderr |
| `--trace-detail` | Also show changed text and structural/rule context |
| `--quiet`, `-q` | Suppress summaries and warnings; errors remain visible |

Trace and line-range modes run the formatter directly. Parallel output is sorted by path. See [cache and tracing](Cache-and-Tracing).

## Generated help

`--help` and `--version` print program information. `--completion SHELL` generates Bash, Elvish, Fish, PowerShell, or Zsh source. `--man` generates a roff manual page. Completion and man options are exclusive and do not require source files or configuration.

```sh
tift --completion bash > tift.bash
tift --man > tift.1
```

## Exit codes and writes

| Code | Meaning |
|---|---|
| `0` | Success or already formatted |
| `1` | Changes detected by check, diff, or fail-on-change |
| `2` | Argument, configuration, input, or formatting error |

Summaries and diagnostics go to stderr. Formatted text, diffs, completion source, and the manual go to stdout. A file with an unsafe structural error remains unchanged; valid batch files continue.

Changed files use a temporary file in the same directory for atomic replacement, preserve permissions, and check that original contents have not changed concurrently. Separate output also checks the destination. BOMs, line endings, and final-newline state are preserved by default.

Backups are created only for changed in-place inputs. `--backup` defaults to one copy. `--backup=N` accepts 1–1000: `.bak` is newest, followed by `.bak.1`, `.bak.2`, and so on. Reducing N retains old files outside the selected slots.
