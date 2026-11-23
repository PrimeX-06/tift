# Rust library API

## Dependency and full-document formatting

```toml
[dependencies]
tift = "0.1.4"
```

```rust
use tift::{Config, FileKind, compile_config, format};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = compile_config(Config::from_toml("[indent]\nwidth = 4\n")?)?;
    let output = format(
        "\\begin{quote}\nHello.\n\\end{quote}\n",
        FileKind::Tex,
        &config,
    )?;
    print!("{}", output.text);
    for diagnostic in output.diagnostics {
        eprintln!("{:?}: {} at {:?}", diagnostic.severity, diagnostic.message, diagnostic.span);
    }
    Ok(())
}
```

Rust 1.99.1 is the minimum version. `Config::default()` selects built-in settings. `Config::from_toml` parses values; `compile_config` and `CompiledConfig::new` validate and compile rules. Reuse compiled configuration across documents.

## Public types

| Type/function | Purpose |
|---|---|
| `FileKind::{Tex, Tikz, Style, Class, Bibtex, Dtx, Ins}` | Select source kind |
| `FileKind::from_path` | Recognize supported extensions case-insensitively |
| `Config` | Serializable typed configuration |
| `Config::default_toml()` | Generate an editable TOML file with built-in settings |
| `CompiledConfig` | Validated configuration with compiled rules |
| `format` | Format a source string |
| `FormatOutput` | Formatted text and diagnostics |
| `Diagnostic` | Severity, message, and source byte span |
| `Severity::{Warning, Error}` | Diagnostic severity |
| `ConfigError` | Configuration failure |
| `FormatError` | Syntax or replacement failure |

Diagnostic spans are byte ranges; respect UTF-8 boundaries when displaying excerpts. Keep warnings visible to the caller when conservative source handling matters.

Structural errors return the original source bytes in `FormatOutput.text` with error diagnostics. A later-stage structural failure rolls the complete file back and discards its edit trace. Replacement execution failures remain `FormatError` results. An unchanged error result counts as unsuccessful formatting.

`source_model::SourceModel::analyze` exposes the immutable source, lexical partition, declarations, signatures and resolved rules used by formatting. `reconstruct()` reproduces the input bytes. `SourceModel::parse_bytes` accepts arbitrary bytes, including invalid UTF-8, and reports encoding errors through a `ByteSourceModel`. See [source analysis](Source-Analysis) for alias inference and rule precedence.

## Selected lines

```rust
use tift::{CompiledConfig, Config, FileKind, LineRanges, format_ranges};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = CompiledConfig::new(Config::default())?;
    let ranges = LineRanges::parse("2-4,!3")?;
    let output = format_ranges(
        "\\begin{quote}\nFirst.\nSecond.\nThird.\n\\end{quote}\n",
        FileKind::Tex,
        &config,
        &ranges,
    )?;
    print!("{}", output.text);
    Ok(())
}
```

Line selections are one-based. Formatting uses full-document structure and widths, then restricts edits. Transformations redistributing text across selected boundaries retain original source and emit warnings.

## Tracing

`layout::format_document_traced` returns `(FormatOutput, Vec<StageTrace>)`. Stages report name, changed status, input/output byte counts, and elapsed duration.

`layout::format_document_detailed` additionally returns `Vec<trace::DetailTrace>` with before/after text, stage-local input/output byte and line ranges, and optional rule/structure context. Detailed line ranges are one-based and end-exclusive. `ranges::format_ranges_traced` combines full-context timing with selection filtering.

## Filesystem responsibilities

The basic library formatting path uses supplied source and configuration without reading configuration files or project imports. CLI configuration discovery, inheritance, file-pattern overrides, dependency context, persistent result files, and writes live in the command-line layer. Applications integrating the library prepare those settings explicitly.

`CompiledConfig::with_declaration_config` and `with_project_syntax` can reuse unchanged compiled formatting rules when supported declaration analysis changes syntax/protection settings. Use these advanced interfaces with a consistent project context.

Generate API documentation from a checkout:

```sh
cargo doc --no-deps --locked --open
```

The library and CLI forbid unsafe Rust code. See the [public interfaces](https://github.com/PrimeX-06/tift/blob/main/src/lib.rs) for additional functionality.
