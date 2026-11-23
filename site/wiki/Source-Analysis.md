# Local definitions and source diagnostics

tift recognizes local command definitions and environment aliases so a document can keep its own shortcuts while receiving consistent indentation, math spacing, and alignment. Definitions in local packages, classes, and included files also contribute to project formatting.

## Environment aliases

Literal zero-argument definitions can open and close environments:

```tex
\def\bea{\begin{equation}}
\newcommand{\eea}{\end{equation}}
\bea
    x = y + z
\eea
```

tift also recognizes zero-argument xparse declarations, indirect aliases, and `\let` copies. Definition order and local scopes determine which definition applies at each use.

Explicit aliases can be configured in `tift.toml`:

```toml
[syntax.environment-aliases.bea]
environment = "equation"
begin = true

[syntax.environment-aliases.eea]
environment = "equation"
begin = false
```

The replacement must resolve to a literal environment boundary. Definitions with parameters, cycles, or uncertain conditional assignments may require explicit configuration. tift reports uncertain aliases and preserves affected content.

## Source preservation

Verbatim content, recorded data, and delayed definitions retain their intended contents. Complete static wrappers can contain nested environments; commands with dynamic environment names receive conservative handling.

Diagnostics refer to locations in the original file. Invalid encoding and incomplete structure produce diagnostics while retaining the input. Preview changes with `tift --diff main.tex`, and inspect effective settings with `tift --show-config main.tex`.

## Rust integration

`tift::source_model::SourceModel` provides source spans, declarations, environment boundaries, and diagnostics for UTF-8 input. `SourceModel::parse_bytes` accepts arbitrary bytes and reports encoding diagnostics. Both forms support exact source reconstruction.

See the [library guide](Library-API) for formatter integration and result handling, and [project context](Project-Context) for local imports and custom syntax.
