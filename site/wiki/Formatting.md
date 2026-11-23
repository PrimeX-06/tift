# Formatting behavior

The [README](https://github.com/PrimeX-06/tift) introduces the core features with a compact input/output example. The [complete formatting gallery](Formatting-Examples) also covers macros, conditionals, custom syntax, replacements, selected lines, and whitespace policies with their TOML settings.

## Indentation

Defaults use four spaces. The `document` body and preamble retain their base level; nested block environments add indentation. Item continuations receive their list context. Rules can select an environment, command, item, special block, or structural category.

```toml
[indent]
width = 2
preamble = true

[indent.rules.quote]
additional = true

[indent.categories.loop]
width = 2
```

`style = "tabs"` uses tabs. `maximum` limits total indentation columns. Rule fields include `additional`, `width`, and a literal whitespace `indent`; named rules take precedence over categories.

Literal `\foreach` loops receive one level per nested loop. Consecutive headers, braced bodies, and unbraced bodies with complete top-level semicolons have explicit scope. `syntax.foreach = false` disables this recognition.

For TikZ data lists, `arguments.foreach-lists = "expand"` puts literal records on separate lines; `"auto"` expands wide lists. The default is `"preserve"`. See [TikZ loop examples](Formatting-Examples#foreach-data-lists).

## Tables and math

Alignment is enabled for common tables, matrices, and aligned math. Complete headers remain together; only top-level cells and rows are measured. Comments, escaped separators, nested groups, protected source, and child environments retain their ownership.

```toml
[alignment]
width = "display"
spaces-before = 1
spaces-after = 1
multicolumn = true

[alignment.rules.'tabular*']
enabled = true

[alignment.rules.matrix]
enabled = false
```

`width = "graphemes"` is the default. `"display"` measures Unicode display columns. Default `justification = "columns"` follows recognized table specifications and accounts for logical `\multicolumn` spans. Rules also support uniform left/center/right justification, short rows, excluded measurements, custom delimiter and row regexes, and row-terminator alignment. See [table column alignment](Table-Alignment) for specifications and spans, and [formula formatting](Math-Formatting) for source spacing and display breaks.

For a custom `tikzcd` separator:

```toml
[alignment.rules.tikzcd]
delimiter = '\\&'
```

Match the separator used by the document's `ampersand replacement` setting. An explicit `alignment.environments` array replaces the built-in selection. Named rules can select custom environments or disable defaults. Same-source redefinitions conservatively preserve custom alignment until explicitly selected.

## Structural boundaries

Ordinary environments place their body and closing token on source lines. Top-level `\\` and `\tabularnewline` row terminators include their star and spacing modifiers. Known block environments start a line; box environments such as `tabular` do so inside floats. Eligible following captions start a line at the environment's level.

```toml
[breaks.rules.quote]
body = "preserve"
end = "preserve"

[breaks.categories.table-rule]
begin = "preserve"
after = "preserve"
```

Set `[breaks] enabled = false` to preserve structural boundaries globally. Name and category rules replace their selected boundary defaults; unspecified values in an explicit rule use `"preserve"`. Math fragments, command parameters, alignment cells, and unknown or locally redefined environments retain conservative external adjacency.

Complete table rules such as `\hline`, `\cmidrule`, and `\specialrule` inherit their environment's row policy. Tight inserted boundaries use empty `%` comments when needed to preserve TeX spacing. Grouped row content such as `\substack` retains its own line layout.

## Prose and sentences

```toml
[wrap]
mode = "wrap"
columns = 88
comments = true

[sentences]
enabled = true
preserve-line-breaks = true
```

`"preserve"` keeps prose line breaks; `"wrap"` keeps existing boundaries and folds overlong lines; `"unwrap"` joins eligible prose; `"rewrap"` joins old prose lines and wraps each paragraph to the configured width. Width includes indentation. Inline math, literal spans, and unsupported argument content remain atomic. Long atomic contents can exceed the width.

```sh
tift --rewrap --wrap-width 88 main.tex
```

Sentence splitting retains old breaks within each sentence by default. Setting `preserve-line-breaks = false` permits joining during sentence-only layout; an active wrapping stage retains the boundaries it receives. `wrap.stage` and `sentences.stage` control stage order.

`comments = true` wraps long standalone and trailing comments with retained markers and indentation. `prose-commands` permits custom commands at prose starts. `text-commands` controls ordinary-text parameter selection; `[]` disables it. `break-chars = ['-']` permits tight width-triggered punctuation breaks with spacing-preserving comments.

## Command arguments

Known key-setting commands such as `\pgfkeys`, `\pgfqkeys`, `\tikzset`, and `\pgfplotsset` can expand at top-level commas. Nested groups, math, escaped separators, comments and verbatim-like data do not become key boundaries. For PGF keys, `.code`, `.style`, `.append code` and `.prepend code` handler bodies remain delayed code while the surrounding key list can be laid out. Automatic mode preserves existing fitting layout and needs an active width constraint. An active prose wrap supplies its width as fallback.

```toml
[arguments]
mode = "auto"
columns = 88
split-equals = true

[arguments.rules.includegraphics]
optional-argument = 0
columns = 60
```

Explicit `mode = "expand"` requests multiline layout; `"preserve"` keeps the selected argument. Brace `argument` and bracket `optional-argument` indices start at zero. Nested selected commands are formatted from the inside out. Groups, escaped separators, math, literal data, and filename content retain their boundaries.

`arguments.align-equals = true` aligns assignment operators in selected multiline lists. It defaults to `false` and can be overridden per command. See the [key/value example](Formatting-Examples#aligning-keyvalue-assignments).

## BibTeX

Default bibliography formatting preserves entry-type and field-name case and aligns assignments. `[bibtex] name-case = "lower"` or `"upper"` selects uniform name case. Entry order, field order, citation keys, brace/quote values, concatenations, and multiline value contents stay as authored. `@string`, `@preamble`, and comment entries have dedicated handling. [BibTeX and BibLaTeX](BibTeX) covers delimiter choices, field ordering, and optional sorting by key, author, year, or custom criteria.

```sh
tift --diff references.bib
tift references.bib
```

Literal value contents remain protected. TeX project declaration scanning is skipped for `.bib` inputs. A field selected by a formatting skip directive preserves its complete entry.

## Whitespace and replacements

Trailing whitespace is removed outside protection. Blank-line runs default to at most two lines. BOMs, LF/CRLF endings, and final-newline state are preserved.

```toml
[whitespace]
max-blank-lines = 1
line-ending = "lf"
final-newline = "always"

[[replacements]]
find = "colour"
replace = "color"
scope = "unprotected"
```

Replacements support literal and bounded regex matching, before/after stages, ordered application, and match limits. `scope = "all"` allows changes inside protected content. See [Configuration](Configuration) for every field.
