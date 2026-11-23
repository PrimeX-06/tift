# Configuration setup

Create and maintain project or personal `tift.toml` settings. The [configuration reference](Configuration) lists all available fields and their defaults.

- [Configuration file name and location](#configuration-file-name-and-location)
- [Configuration discovery and explicit files](#configuration-discovery-and-explicit-files)
- [Personal configuration locations](#personal-configuration-locations)
- [TOML syntax and writing rules](#toml-syntax-and-writing-rules)
- [A practical project configuration](#a-practical-project-configuration)
- [Inheritance and per-file overrides](#inheritance-and-per-file-overrides)
- [Priority and file-pattern rules](#priority-and-file-pattern-rules)
- [Inspect and validate settings](#inspect-and-validate-settings)
- [Configuration in VS Code](#configuration-in-vs-code)

## Configuration file name and location

The automatically discovered filename is **`tift.toml`**. Save it as a UTF-8 text file in your LaTeX project, usually beside `main.tex`. Configuration is optional; missing settings use built-in defaults.

```text
paper/
├── .git/
├── tift.toml
├── main.tex
├── chapters/
│   ├── introduction.tex
│   └── results.tex
├── slides/
│   └── talk.tex
└── figures/
```

In this layout, formatting `main.tex`, `chapters/introduction.tex`, or `slides/talk.tex` selects `paper/tift.toml`. Run the following from `paper/`:

```sh
tift --show-config main.tex
tift --diff chapters/introduction.tex
tift .
```

You can start with a short file:

```toml
[indent]
width = 4

[wrap]
mode = "preserve"
```

Or copy the repository's [annotated example](https://github.com/PrimeX-06/tift/blob/main/examples/tift.toml) to your project as `tift.toml`, then enable the settings you need. That example demonstrates optional rules as well as basic defaults.

## Configuration discovery and explicit files

For each input, the CLI starts in that input's directory and searches upward for the nearest `tift.toml`. It stops at a Git root or the filesystem root. A batch can therefore use different configurations for files in different subdirectories.

An additional `slides/tift.toml` takes over for files under `slides/`. Its unspecified values use built-in defaults. Use `extend = "../tift.toml"` to inherit the parent configuration explicitly.

With no project file, the CLI falls back to a personal configuration. An explicit file can have any name:

```sh
tift --config configs/journal.toml main.tex
tift --config /path/to/shared/style.toml chapters/
tift --no-config --indent-width 2 main.tex
```

`--config` selects the given file instead of automatic discovery. Its relative path is resolved from the CLI working directory. `--no-config` selects built-in defaults plus CLI flags. The two options are mutually exclusive.

For editor stdin, the virtual filename supplies the directory used for discovery and the path used for overrides:

```sh
tift - --stdin-filename /path/to/paper/chapters/introduction.tex
```

## Personal configuration locations

Personal settings provide a fallback for projects without `tift.toml`:

| Platform | Default personal file |
|---|---|
| Windows | `%APPDATA%/tift/tift.toml`; fallback `%USERPROFILE%/AppData/Roaming/tift/tift.toml` |
| Linux / Unix | `$XDG_CONFIG_HOME/tift/tift.toml`; fallback `$HOME/.config/tift/tift.toml` |
| macOS | `$XDG_CONFIG_HOME/tift/tift.toml` when set; otherwise `$HOME/Library/Application Support/tift/tift.toml` |

Set **`TIFT_CONFIG_HOME` to a directory** to select `<directory>/tift.toml`. An empty value disables personal discovery. Project configuration, `--config`, and `--no-config` take precedence over this fallback. To combine project and personal settings, point `extend` at the personal file explicitly.

Cache storage has separate locations and uses `TIFT_CACHE_HOME`; see [cache and tracing](https://github.com/PrimeX-06/tift/wiki/Cache-and-Tracing).

## TOML syntax and writing rules

Use `[section]` tables, `key = value` assignments, `#` comments, and arrays in square brackets:

```toml
# Root-level options belong before the first table.
extend = "base.toml"

[indent]
style = "spaces"              # String
width = 4                     # Integer
preamble = false              # Boolean

[files]
exclude = ["generated/", "vendor/"] # Array of strings

[alignment.rules.'tabular*']
enabled = true                # Quote names containing punctuation.
```

Field names use **kebab case**, such as `line-ending` and `spaces-before`. Values are typed: booleans use `true`/`false`, numbers use numeric literals, and strings use quotes. A key following `[wrap]` belongs to that table until another table begins; put the root key `extend` before the first table. The example above expects a `base.toml` file beside `tift.toml`; omit `extend` for a standalone configuration.

Single-quoted TOML strings retain backslashes literally and are useful for Windows paths and TeX command names.

Double-quoted strings interpret TOML escapes. Forward slashes also work in Windows path strings, for example `"C:/Projects/shared/style.toml"`. `[[overrides]]` creates an array of tables; each repeated heading starts a new entry. Ordinary tables should have one definition, with their settings grouped together.

Unknown fields, wrong types, invalid enum values or patterns, and inheritance cycles produce errors. Every override is validated, including entries that do not match the current input.

## A practical project configuration

This configuration keeps prose boundaries, uses four-space indentation and Unicode display-width alignment, and excludes generated sources:

```toml
[indent]
style = "spaces"
width = 4
preamble = false

[alignment]
enabled = true
width = "display"
spaces-before = 1
spaces-after = 1

[wrap]
mode = "preserve"
columns = 88

[whitespace]
remove-trailing = true
max-blank-lines = 2
line-ending = "preserve"
final-newline = "preserve"

[files]
exclude = ["generated/", "vendor/", "*.generated.tex"]
```

Change `wrap.mode` to `"wrap"` to fold overlong prose at 88 columns, or `"unwrap"` to join eligible prose lines. `wrap.columns` alone preserves the selected mode. Active wrapping also supplies its width to command-argument layout, including [image options](Formatting-Examples#nested-keyvalue-and-image-options).

Full field defaults, named/category rules, custom signatures, regex options, and source-protection settings are listed in the [configuration reference](Configuration).

## Inheritance and per-file overrides

To share settings, put a base file beside `tift.toml`:

```text
paper/
├── base.toml
├── tift.toml
├── main.tex
├── slides/talk.tex
└── chapters/results.tex
```

**base.toml**

```toml
[indent]
width = 4

[wrap]
mode = "preserve"

[files]
exclude = ["generated/"]
```

**tift.toml**

```toml
extend = "base.toml"

[files]
exclude = ["generated/", "vendor/", "*.generated.tex"]

[[overrides]]
files = ["slides/**/*.tex"]

[overrides.indent]
width = 2

[[overrides]]
files = ["chapters/**/*.tex"]

[overrides.wrap]
mode = "wrap"
columns = 88
```

`main.tex` uses four-space indentation and preserved prose. Slides use two spaces. Chapters keep the base indentation and enable wrapping. Several matching overrides apply in declaration order.

For a separate `slides/tift.toml`, inherit the parent explicitly:

```toml
extend = "../tift.toml"

[indent]
width = 2
```

Tables merge recursively. Arrays replace inherited arrays, including exclusions, environment lists, raw protection lists, and override arrays. Include every array entry you want to retain. An inherited override array remains active when the selected configuration does not supply a replacement array. Overrides support partial normal configuration tables; nested overrides and `extend` inside an override are rejected.

## Priority and file-pattern rules

Settings apply in this order, with later settings taking precedence:

1. Built-in defaults.
2. The `extend` chain, from the oldest base to the nearest base.
3. The selected configuration's own fields.
4. Matching `[[overrides]]`, in declaration order.
5. Command-line options.

For example, `tift --indent-width 3 slides/talk.tex` selects three spaces even if an override selects two.

`extend`, exclusion patterns, override patterns, and configured cache paths are relative to the configuration file declaring them. Inherited patterns retain their original directory. Patterns address descendants of that directory; placing shared patterns in a base under `configs/` limits them to descendants of `configs/`.

File patterns use Git ignore syntax: `*`, `**`, directory patterns, leading `/` anchors, and `!` negation. Directory discovery also respects `.gitignore` and `.ignore`. Explicit file arguments bypass directory ignore rules while retaining `files.exclude`. Symlinks are skipped or rejected, and duplicate paths are processed once.

## Inspect and validate settings

```sh
tift --show-config main.tex
tift --show-config slides/talk.tex
tift --config tift.toml --show-config chapters/results.tex
tift --diff main.tex
```

`--show-config` prints merged TOML, configuration sources, matched override count, and exclusion status. It accepts a virtual path without requiring the source to exist. Root comments and source declarations are prepared during actual formatting.

Generate a complete starting configuration with the installed formatter:

```sh
tift --dump-config > tift.toml
```

The command writes TOML to stdout and does not read the current project configuration. Package-derived environment lists remain supplied by built-in rules; an explicit list in `tift.toml` replaces that list. Use `tift --show-config main.tex` to inspect the effective configuration for a file.

## Configuration in VS Code

LaTeX Workshop's temporary formatting file is in the document directory, so ordinary `tift.toml` discovery follows the same project hierarchy. Filename-specific overrides see the adapter's temporary name. The stdin API with `--stdin-filename` can retain the actual document path for integrations that support it.

To select a fixed configuration through the LaTeX Workshop adapter, set its argument array to:

```json
[
    "--print",
    "--config", "C:/Projects/paper/tift.toml",
    "%TMPFILE%"
]
```

See the [VS Code guide](https://github.com/PrimeX-06/tift/wiki/VS-Code) for the complete settings object and executable setup.
