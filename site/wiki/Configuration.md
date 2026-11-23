# Configuration reference

Start with the [configuration setup guide](Configuration-Guide) for the `tift.toml` filename, project and personal locations, TOML syntax, a complete project example, inheritance, overrides, and configuration inspection.

Configuration files are UTF-8 TOML. Field names use kebab case. Unspecified values inherit built-in defaults. Generate an editable default file with `tift --dump-config > tift.toml`; [annotated configuration](https://github.com/PrimeX-06/tift/blob/main/examples/tift.toml) contains a shorter practical example.

Settings are grouped by responsibility: layout (`indent`, `whitespace`, `breaks`, `wrap`, `sentences`, `arguments`, `alignment`, `math`, `trees`, `code`); source recognition (`syntax`, `protect`); bibliography (`bibtex`); project selection (`files`, `overrides`, `extend`); runtime (`cache`); text edits (`replacements`, `replacement-only`). Package-derived environment lists are omitted from generated TOML and stay dynamic. An explicit list replaces the built-in and package-provided entries.

## Indentation

| Field | Default | Behavior |
|---|---|---|
| `indent.style` | `"spaces"` | `"spaces"` or `"tabs"` |
| `indent.width` | `4` | Spaces per level; valid range is 1–256 |
| `indent.preamble` | `false` | Format structural indentation inside the preamble |
| `indent.maximum` | unset | Limit total indentation columns; tabs count as `indent.width` |
| `indent.categories.<category>` | category defaults | Indentation rule for a structural category |
| `indent.rules.<name>` | name defaults | Rule for a named environment, command, item, or special block |

Each rule accepts `additional`, `width`, and `indent`. `additional = false` suppresses the block's extra indentation. `indent` supplies a literal whitespace unit and accepts spaces and tabs. A name rule takes precedence over its category rule. `document` defaults to `additional = false`; command and ordinary group category wrappers also have no additional level, while their argument bodies receive their own level.

Categories are `environment`, `verbatim`, `command`, `mandatory-argument`, `optional-argument`, `group`, `named-group`, `key-value`, `condition`, `loop`, `tikz-path`, `item`, `heading`, and `special`. Math delimiters use special blocks named `displayMath`, `inlineMath`, `$`, or `$$`. BibTeX entry indentation uses the `bibtex` category and the entry type as its name.

## TikZ paths and loops

TikZ paths use `tikz-path` for continuation indentation and structural break settings. Path command names such as `draw` and `node` accept individual overrides. The default separates complete statements at their outer semicolons; options, coordinates and node text keep their internal syntax. `[indent.categories.tikz-path] additional = false` disables the extra continuation level. An explicit `[breaks.categories.tikz-path]` rule replaces automatic path boundaries; setting `begin` and `after` to `"preserve"` retains their original line boundaries.

Recognized `\foreach` loops use category `loop` and name `foreach`. Each loop adds one level, including consecutive nested loop headers. Its paired brace body shares the loop's indentation unit. `indent.rules.foreach` takes precedence over `indent.categories.loop`; `additional = false` suppresses the loop's extra level.

TikZ `\foreach` data lists use `arguments.foreach-lists`. The default `"preserve"` retains their authored layout; `"auto"` expands lists exceeding `arguments.columns` or the inherited `wrap.columns`; `"expand"` puts literal records on separate lines. Slash-separated records, tuples, grouped values, and compact ranges retain their boundaries. See [TikZ loop examples](Formatting-Examples#foreach-data-lists).

## Alignment

Alignment defaults to enabled for common table, matrix, and aligned math environments. Package defaults include starred tables, xtab/supertabular variants, tabularray, NiceTabular/NiceArray/NiceMatrix, subarrays, multlined/gathered fragments, extended alignat environments, fenced small matrices, and `tikzcd`. TikZ `\matrix` command bodies also use default matrix alignment. The exact command parser keeps options, node names, and positions in the header and aligns the final brace body. A named `alignment.rules.<name>` entry enables a custom environment or another supported named block and can override individual options.

| Field | Default | Behavior |
|---|---|---|
| `enabled` | `true` | Enable the alignment pass |
| `width` | `"graphemes"` | Grapheme count or Unicode `"display"` columns |
| `environments` | built-in list | Array of environment names |
| `delimiter` | `'&'` | Regex for a cell separator |
| `row-break` | `'\\\\\*?|\\tabularnewline\*?'` | Regex for a row terminator |
| `delimiter-group`, `row-break-group` | unset | Capture group containing the separator; consumed context is preserved |
| `regex-engine` | `"enhanced"` | `"rust"` linear regex or `"enhanced"` with lookaround/backreferences |
| `backtrack-limit` | `1000000` | Enhanced regex backtracking budget |
| `spaces-before`, `spaces-after` | `1` | Spacing around delimiters |
| `spaces-before-row-break` | `1` | Spacing before row terminators |
| `align-row-breaks` | `true` | Align row terminators |
| `align-last-row-break` | `false` | Include the final row terminator |
| `align-content-after-row-break` | `false` | Align content after row terminators |
| `spaces-after-row-break` | `1` | Spacing before trailing content |
| `format-children` | `true` | Format nested eligible alignment blocks |
| `column-spec` | `"preserve"` | `"auto"` expands wide column preambles; `"expand"` puts each logical column on its own source line |
| `multicolumn` | `false` | Interpret `\multicolumn` logical column spans; `justification = "columns"` enables spans automatically |
| `short-rows` | `"align"` | `"align"` or `"preserve"` |
| `justification` | `"columns"` | Literal column-spec alignment by default; uniform cell `"left"`, `"right"`, or `"center"` alignment |
| `delimiter-justification` | `"left"` | `"left"`, `"right"`, or `"center"` alignment of variable-width separator matches |
| `dont-measure` | `[]` | Cell patterns omitted from width measurement |
| `dont-measure-rows` | `[]` | Row patterns omitted from measurement |
| `dont-measure-literals` | `[]` | Exact cell strings omitted from measurement |
| `dont-measure-largest` | `false` | Exclude the largest measured cells |

All fields above belong under `[alignment]`. A named rule accepts the corresponding partial options. Field names are exact; unknown spellings produce a configuration error. See [table column specification layout](Table-Alignment#column-specification-layout) for a complete example of `column-spec`.

`alignment.enabled = false` disables the alignment pass. An explicit `environments` array replaces the default selection; named rules can enable or disable individual supported blocks. The name `matrix` controls both matrix environments and TikZ matrix command bodies. Quote starred names in TOML:

```toml
[alignment.rules.matrix]
enabled = false

[alignment.rules.'tabular*']
enabled = false
```

Source declarations redefining `\matrix`, or an explicit `syntax.commands.matrix` entry, preserve custom matrix command bodies unless a named alignment rule explicitly selects them. `format-children = false` preserves nested eligible blocks while allowing outer alignment. Literal regions and protected macro bodies keep their protection. See [default table and matrix alignment](Formatting) for examples.

Known package environments redefined in the current source retain their custom alignment layout by default. An explicit named alignment rule can select the environment. Wrapper environments such as `NiceMatrixBlock`, `subequations`, and `spreadlines` provide scope for their contents; their child matrices and equations use their own alignment rules. For `tikzcd` with an alternative cell separator, such as `ampersand replacement=\&`, configure that separator with a named `delimiter` rule.

Comments, escaped separators, nested groups, literal spans, and child environments are opaque while locating top-level delimiters. Marked blocks such as `%* \begin{tabular}` / `%* \end{tabular}` retain their markers.

The default `justification = "columns"` aligns source cells according to each table's literal column preamble. `l/L`, `c/C`, and `r/R` choose left, center, and right alignment; `p{...}`, `m{...}`, and `b{...}` use left alignment. `X/Q` default to left and `S/s` numeric columns to center across supported table families. Complete literal local column declarations override these defaults. The parser accepts `*{n}{...}` repetition, vertical rules, and `>`, `<`, `@`, and `!` modifiers while preserving their TeX bodies. Horizontal `l/c/r/j` and `halign` options inside `X[...]` and `Q[...]` select their source justification. Tabularray reads `colspec={...}` from the inner options and applies row, column, and cell settings in source order. Top-level `SetCell/SetRow/SetColumn/SetRows/SetColumns/SetCells` overrides use logical coordinates and span-aware measurement. Known table/array header signatures locate the exact preamble after placement, width, and delimiter arguments.

In this mode, `\multicolumn{n}{spec}{content}` occupies its logical span and uses a supported one-column override. Unknown overrides retain the span and use the starting column's alignment. Unknown column arity, malformed or excessive repetition, unsupported column definitions, and custom environment signatures use left alignment for that preamble. Complete literal local column aliases retain their inferred horizontal alignment. Arbitrary TeX macros in column specifications are preserved without expansion. A named rule can select uniform alignment for one table family:

```toml
[alignment.rules.tabular]
justification = "left"
```

## Prose and sentences

| Field | Default | Behavior |
|---|---|---|
| `wrap.mode` | `"preserve"` | `"preserve"`, existing-line wrapping `"wrap"`, joining `"unwrap"`, or merge-then-wrap `"rewrap"` |
| `wrap.columns` | `80` | Target width, including indentation |
| `wrap.stage` | `"before"` | `"before"` or `"after"` text-layout stage |
| `wrap.comments` | `false` | Reflow comment-only lines and long trailing comments; `wrap` retains old boundaries and `rewrap` joins matching comment lines |
| `wrap.prose-commands` | `[]` | Additional command names eligible at the start of prose lines |
| `wrap.text-commands` | unset | Use safe built-in ordinary-text parameter selection during wrapping; an explicit list limits selection and `[]` disables it |
| `wrap.break-chars` | `[]` | Additional single characters after which width-triggered prose breaks are allowed |
| `wrap.collapse-spaces` | `true` | Collapse ordinary horizontal whitespace during reflow |
| `wrap.begin-pattern` | unset | Regex defining eligible prose block starts |
| `wrap.end-before-pattern` | unset | Regex before which a prose block ends |
| `wrap.follow-pattern` | unset | Regex matching the preceding line's end |
| `wrap.separator` | `""` | Use source line endings, or explicit LF/CRLF/comment separators |
| `wrap.long-words` | `"overflow"` | Keep long words intact, or `"break"` at grapheme boundaries |
| `sentences.enabled` | `false` | Split prose at sentence boundaries |
| `sentences.preserve-line-breaks` | `true` | Retain existing line breaks within a sentence; `false` permits joining outside an active wrap stage |
| `sentences.stage` | `"before"` | Sentence-layout stage |
| `sentences.abbreviations` | common English abbreviations | Array replacing the built-in abbreviation list |
| `sentences.begin-pattern` | unset | Regex selecting a following sentence start |
| `sentences.end-pattern` | built-in punctuation | Regex selecting sentence endings |
| `sentences.indent` | `""` | Whitespace added to separated sentence lines |
| `sentences.exclude-pattern` | unset | Exclude matching prose blocks from sentence changes |

Custom text regexes use the linear engine when supported and the enhanced engine otherwise, with a backtracking limit of 1,000,000. Invalid patterns are rejected when compiling configuration. Inline math, literal content and unsupported command arguments remain atomic. Known ordinary-text parameters may wrap at existing ASCII whitespace; this text pass preserves short captions, color metadata and filename contents. The separate argument pass can expand selected option lists. Comment-suppressed source connections retain their spacing semantics.

Register custom prose commands with `prose-commands = ["mytext"]`; a leading backslash and a trailing star are accepted. Names match complete commands and supplement the built-in prose command list. `begin-pattern` can also explicitly qualify a custom command. Input paragraphs and generated continuation lines share this eligibility check.

For `break-chars = ['-']`, a long `alpha-beta-gamma-...` string can break after a hyphen when it exceeds the width. A tight source boundary receives `%` before the newline, preserving the original TeX text without an added space. Existing generated boundaries remain in place during wrapping. Nested arguments, math, and literal spans remain atomic at these tight punctuation breaks. Each entry must be one non-whitespace character; TeX syntax characters such as `%` and `\` are rejected. The generated `%` is outside the prose display-column budget, so a source line can exceed that budget by one character. `unwrap` removes empty line-end `%` comments and joins the following prose without introducing whitespace. Existing whitespace before `%` is retained. Comments containing text, escaped percent signs, control-word boundaries, and paragraph boundaries retain their meaning.

`wrap.mode = "rewrap"` or `--rewrap` joins eligible old prose lines in each paragraph, then wraps the result at `wrap.columns`. Blank paragraphs, list items, structural blocks, comments containing text, protected regions, and multiline math/argument atoms keep their boundaries. Safe empty line-end `%` seams retain TeX word adjacency when joined. The mode also joins eligible old lines in selected ordinary-text arguments such as captions, preserving their blank paragraph boundaries. With sentence splitting enabled, old physical line boundaries are removed and each generated sentence boundary is retained during wrapping. `comments = true` applies the same merge-then-wrap behavior to matching comment-only paragraphs.

With `comments = true`, long trailing comments keep the code and first `%` in place and continue on comment-only lines using the same indentation and marker. Existing comment line boundaries remain in place during wrapping. Empty comments, changed markers, and changed indentation end the paragraph. A standalone short trailing comment retains its source bytes. `abc% explanation` continues to connect `abc` tightly to the next source line.

Prose line breaks are preserved by default, including Chinese prose mixed with inline math. `wrap.columns` and `--wrap-width` only set the target width; enable reflow with `wrap.mode = "wrap"` / `--wrap`, or `wrap.mode = "rewrap"` / `--rewrap`. For example, `tift --rewrap --wrap-width 88 paper.tex` joins old prose lines and wraps them at 88 columns. Structural environment line breaks have their own configuration below. See [the prose behavior comparison](Formatting) for the width calculation and a mixed Chinese example.

`wrap.mode = "wrap"` or `--wrap` keeps existing prose line boundaries and wraps only lines that exceed the width. Sentence splitting preserves sentence-internal old line boundaries by default, including when wrapping is also enabled. With sentence splitting alone, set `sentences.preserve-line-breaks = false` to permit joining them; an active wrap stage independently retains the boundaries it receives. The stages run in order, and an active final text stage carries forward wrapping or sentence layout enabled before it. Command parameter wrapping inherits the same column budget; known ordinary-text parameters and `includegraphics` option lists have conservative automatic selection, while unsupported syntax retains its source layout. See [wrapping refinements](Formatting) and [nested argument layout](Formatting).

## Command argument layout

Argument layout operates on selected brace arguments of commands such as `\pgfkeys`, `\tikzset`, and `\pgfplotsset`, or explicitly selected optional bracket arguments. Defaults preserve their layout unless an active width constraint requires expansion.

| Field | Default | Behavior |
|---|---|---|
| `arguments.mode` | `"auto"` | `"auto"`, `"preserve"`, or explicit `"expand"` |
| `arguments.columns` | unset | Positive display-column budget, including indentation and surrounding source |
| `arguments.commands` | `pgfkeys`, `tikzset`, `pgfplotsset`, common package key-setting commands, and expl3 key lists | Selected command names without a backslash |
| `arguments.split-commas` | `true` | Expand at commas owned by the selected argument |
| `arguments.split-equals` | `false` | Allow a break after the first top-level equals in an item |
| `arguments.align-equals` | `false` | Align assignment operators in multiline literal key/value lists; overridable per command |
| `arguments.rules.<name>.argument` | `0` | Zero-based brace argument index, below 256 |
| `arguments.rules.<name>.optional-argument` | unset | Select a zero-based optional bracket argument instead, below 256; requires a known signature or explicit optional syntax |

Named rules also accept `mode`, `columns`, `split-commas`, `split-equals`, and `align-equals` as overrides. A named rule selects its command even when absent from `commands`. Width precedence is the named rule, `arguments.columns`, then `wrap.columns` when `wrap.mode` is `"wrap"` or `"rewrap"`. `wrap.columns` or `--wrap-width` alone does not activate ordinary argument expansion. `auto` without a width retains existing line boundaries; `align-equals` can align existing multiline assignments. `preserve` always retains layout. `expand` explicitly selects multiline layout without needing a width.

See [aligning key/value assignments](Formatting-Examples#aligning-keyvalue-assignments) for input and output using `align-equals`.

With `wrap` or `rewrap`, `includegraphics` and its starred form use their first optional argument as an automatic key/value layout target. The source-line budget inherits `wrap.columns` and includes the command prefix and text after the selected group; calls whose existing lines fit retain their layout. Filenames retain their content. Explicit rules replace the automatic selection: include `optional-argument = 0` when overriding option width or forcing option expansion. A `preserve` rule disables automatic expansion. Nested selected command arguments are laid out from the inside out, retaining each command's own mode, width and protected ranges. Explicit `expand` requires an available comma or enabled equals break; indivisible single items retain their layout.

```toml
[arguments]
columns = 88
split-equals = true

[arguments.rules.customkeys]
argument = 1
columns = 60
```

At 25 columns, `\pgfkeys{alpha={one}, beta={two}, gamma={three}}` becomes:

```tex
\pgfkeys{%
    alpha={one},
    beta={two},
    gamma={three}%
}
```

Expansion considers the command prefix, indentation, Unicode display width, tab stops, and suffix on the closing line. Equals breaks apply only when the individual item exceeds the selected width, or when `expand` explicitly forces them. Nested groups, optional groups, math, escaped punctuation, and literal spans own their contents and keep their internal separators. The value continuation uses the selected argument's structural indentation. Long atomic values can still exceed the budget.

Tight generated boundaries use `%` to preserve TeX token spacing. Empty comments can be reprocessed at a narrower width when doing so preserves token boundaries. Comments containing text, blank paragraphs, and empty-comment boundaries that separate a control word from following letters retain their argument layout. Explicit `syntax.commands` arity and optional-argument settings limit argument selection. Eligible nested command arguments are rendered from the inside out, with each command's own layout rule. Custom rules select declared brace or optional bracket groups; arbitrary command execution and unbraced parameter grammars require explicit syntax and retain conservative handling.

## Structural line breaks

`breaks.enabled` defaults to `true`. Ordinary environments use `body = "line"`, `end = "line"`, and `row = "line"`. Their default begin placement follows structure: known block environments start a source line, and box environments such as `tabular` and `minipage` start a line inside block containers such as `table` and `figure`. Math fragments, command arguments, alignment cells, unknown environments, and locally redefined environments keep their external adjacency. Eligible following captions start a source line; other `after` boundaries are preserved. `row` applies after complete top-level `\\` and `\tabularnewline` terminators, including their star and `[spacing]` modifiers. Brace groups retain their internal row layout, while nested explicit environments use their own row rule. The `document` environment preserves rows by default, as does text outside environments. Loops and table rules use the defaults described below; other structural categories have no built-in boundary changes. Rules live in `breaks.categories.<category>` and `breaks.rules.<name>`. A name rule takes precedence over its category; an explicit category rule takes precedence over built-in defaults. Each selected rule supplies its own boundary values, with unspecified values set to `"preserve"`.

The complete `\caption{Title}` before a `tabular` inside a float gets a plain source line break. Bare control words also permit a plain break because TeX discards their following whitespace. Tight text or an unknown argument-bearing command gets `%` before the new line to preserve spacing. Caption signatures identify the exact argument endpoint, and literal redefinitions in declarations or wrapper bodies disable that paragraph assumption. Explicit `begin = "preserve"`, `begin = "line"`, and `begin = "comment-line"` override automatic placement. See [environment placement examples](Formatting).

Eligible environment endings followed by an exact `\caption`, `\captionof`, or `\subcaption` command, including starred forms, put the caption at the ending environment's indentation level. The rule uses the same structural eligibility as automatic begin placement. Locally redefined or explicitly configured caption commands use an empty comment at tight boundaries to retain spacing. Explicit environment name or category rules control `after` placement and take precedence over this default. See [environment endings and captions](Formatting).

Recognized loops default to `begin = "line"`, `body = "line"`, `end = "line"`, and `after = "line"`, with `row = "preserve"`. Tight inserted boundaries use empty comments to preserve TeX whitespace. A loop body's first top-level semicolon closes an unbraced loop; braces and consecutive nested loops use their parsed scope. To retain loop boundaries and choose a smaller indentation unit:

```toml
[breaks.categories.loop]
begin = "preserve"
body = "preserve"
end = "preserve"
after = "preserve"

[indent.categories.loop]
width = 2
```

`breaks.rules.foreach` and `indent.rules.foreach` override their category settings. `syntax.foreach = false` disables dedicated loop recognition while general formatting remains active.

Row boundaries run before indentation and independently of column alignment. To retain compact matrix rows while still splitting the header and closing token:

```toml
[breaks.rules.matrix]
body = "line"
end = "line"
row = "preserve"
```

Trailing comments stay with their row. Modifier lookahead accepts whitespace and comments across physical lines, and stops at a blank paragraph. Unknown or locally redefined environments use an empty comment for a new boundary tightly adjacent to text. See [row boundary examples](Formatting) for the source comparisons and checks.

Complete table rules at the top level of an eligible table receive boundaries before the command and after its arguments. Recognition covers standard rules, booktabs rules and trims, hhline/makecell rules, longtable header/footer markers, and tabularray-specific rule arguments. For example, `\cmidrule[.3pt](lr){1-2}` stays together, and a following `{a}` remains the first cell. Nested groups and command arguments retain their own layout.

Table-rule `begin` and `after` boundaries inherit the containing environment's `row` mode. Use the `table-rule` boundary category to choose their placement independently, or the exact command name for one rule:

```toml
[breaks.categories.table-rule]
begin = "preserve"
after = "preserve"

[breaks.rules.cmidrule]
begin = "line"
after = "line"
```

Named command settings take precedence over the category. An environment's `row = "preserve"` preserves automatic table-rule boundaries when no explicit table-rule boundary setting applies; a named or `table-rule` category rule can override that default. Global `breaks.enabled = false` disables the pass; alignment remains separately configurable. Literal source, same-source redefinitions, explicit command syntax, and `filecontents` constrain automatic rule recognition. Table rules keep the containing environment's indentation level.

Set `breaks.enabled = false` to disable all structural boundary changes. `--modify-line-breaks` enables the pass when configuration has disabled it. To retain the source boundary layout for one environment:

```toml
[breaks.rules.matrix]
body = "preserve"
end = "preserve"
```

| Boundary | Position |
|---|---|
| `begin` | Before the opening token |
| `body` | After the complete environment header; after the opening token for other block kinds |
| `end` | Before the closing token |
| `after` | After the closing token |
| `row` | After a complete top-level row terminator and its modifiers |
| `else-begin`, `else-after` | Before/after a conditional `\else` |
| `or-begin`, `or-after` | Before/after a conditional `\or` |

Boundary values are `"preserve"`, `"join"`, `"line"`, `"comment-line"`, `"blank-line"`, and `"reset-blank-line"`. Comment-owned newlines and protected contents constrain edits that could change source meaning.

Recognized environment headers include optional positioning, widths, and column specifications. For example, the default body boundary follows all of `\begin{tabularx}{\linewidth}[t]{lc}`. Ordinary `matrix` has no header arguments, so its first `{a}` or `[a,b]` remains in the body; `matrix*[r]` keeps `[r]` in the header. Existing newlines and comments inside a header are retained.

Built-in package signatures describe the complete ordered header. Examples include two optional arguments for `multlined`, a mandatory alignment specification for `subarray`, a token-adjacent final option for NiceArray/NiceMatrix, and a parenthesized configuration for `forest`. The first bracketed forest tree belongs to the body. Known column specifications stay protected during structural scanning, including nested definitions and conditionals, so column-template syntax keeps its original bytes.

Subcaption's `subfigure`, `subtable`, and `subcaptionblock` use the complete `ooom` header: optional placement, height and inner placement, followed by the mandatory width. For example, `\begin{subfigure}[t][3cm][c]{0.46\linewidth}` stays together before the body boundary. `subcaptiongroup` and `subcaptiongroup*` have no header arguments. Explicit `syntax.environment-arguments` entries and recognized local environment definitions take precedence.

OCG, animation, graphics and science environments have built-in ordered headers. `ocg` uses `ommm`, `ocmd` and `animateinline` use `om`, and chemmacros reaction environments use token-adjacent `O`. A spaced `[Na]` starts reaction content. `pgfpicture` selects four mandatory legacy dimensions when its first token is a brace, and zero parameters otherwise. Delayed animation begin/end hooks remain argument data. Plural `reactions` and `psmatrix` receive default alignment; drawing and chemistry bodies stay outside prose wrapping. Raw Asymptote, Gnuplot and OCG-table readers preserve package-specific end boundaries and honor scoped redefinitions. The full registry and examples are in [graphics and science environments](Formatting).

The commands `\subfigure`, `\subtable`, and `\subfloat` keep up to two optional list-entry/caption arguments opaque to outer table and row scans. `\subcaptionbox` keeps its optional list entry, required caption and optional width/inner-position arguments opaque; its starred form omits the list entry. Their final content argument remains available for ordinary group and nested-environment formatting. Recognized local redefinitions and explicit `syntax.commands` entries control these defaults. Explicit literal command protection can preserve the whole invocation.

An explicit literal `\documentclass{elegantbook}`, `elegantpaper`, or `elegantnote` selects the common environments defined by that class. Modern ElegantBook fancy theorems keep optional braced and bracketed titles, an exact `\label` token, and the optional braced label in the header. Starred variants omit the label slots. `mode=simple` and the `simple` alias select amsthm optional titles, as used by Paper and Note. Book's `custom` consumes one mandatory title; its introduction, problemset, and change wrappers supply list context while `syntax.infer-list-environments` is enabled. Existing item lines and their continuations use ordinary item indentation; explicit item boundary rules control added line breaks.

Class grammar inference skips protected or nested fake declarations and respects local environment redefinitions and explicit `syntax.environment-arguments` settings. External `\PassOptionsToClass` and top-level `\ekv` Book settings use conservative handling. The library does not inspect external class files or infer their versions. Older ElegantBook releases with mandatory title and label arguments require explicit settings.

Under normal catcodes, the built-in boundary policy for unknown environments uses `"comment-line"` at tightly adjacent body/end boundaries to preserve TeX whitespace semantics. Existing whitespace uses a normal line break. Explicit name or category rules use their requested modes. Verbatim and other protected regions retain their boundaries; `filecontents` environments named in `protect.file-contents` retain their complete source, including tabs, trailing spaces and blank lines. Set `protect.file-contents = []` to allow formatting within them. Dynamic tokenization requires source protection. See [environment boundary behavior](Formatting) for examples and semantic checks.

## Syntax and protection

`syntax.items` and `syntax.list-environments` define list commands and their eligible environments. `syntax.conditions` adds conditional command names. `syntax.headings.<name>` defines `level` and whether the heading body adds indentation. `syntax.commands.<name>` accepts `arguments`, `optional-arguments`, `signature`, `verbatim`, and `preserve-body`. `signature` supplies a complete xparse argument specification and replaces the legacy argument fields. Defaults and processor bodies retain their source tokens.

```toml
[syntax.commands.annotate]
signature = 's t! e{^_} O{default} >{\TrimSpaces} D(){none} m'
```

TeX primitives receive handling specific to their operands. `\string`, `\meaning`, `\ifx`, `\ifdefined`, `\ifprimitive`, and `\let` read their token data. `\noexpand` suppresses structural recognition for a consumed control sequence; ordinary braces retain their grouping role. `\if` and `\ifcat` precisely consume two direct character operands; control-sequence operands requiring expansion receive conservative handling. `\futurelet` reads its assignment target as data, while its two lookahead tokens remain in the input for execution. Braced `\detokenize` and `\showtokens` data, and the target and parameter tokens in definition headers, retain their data role. Adjacent inline formulas such as `$a$$b$` are recognized with the current math state.

Literal `\loop...\repeat` closes its final unfinished conditional. Groups and conditionals can finish in either order. Known `\iftrue`, `\iffalse`, literal `\ifnum`, `\ifdim`, `\ifodd`, and literal-integer `\ifcase` preserve every skipped branch as opaque source. Skipped branches can contain unbalanced math, groups, and environments; their nested conditional tokens are still matched. Active branches receive normal structural validation. `\unless` inverts known outcomes, including numeric comparisons. Signed decimal, octal, hexadecimal and literal character integers are recognized. Literal dimensions use TeX scaled-point conversion. Unknown numeric operands retain dynamic structural analysis. Recognized `\let` aliases use their active bindings, including local scope; assignments under unknown conditions retain unknown meanings.

For a condition whose result is unknown, branches choosing different environments preserve the smallest interval until the possible structural stacks agree again. Independently balanced branch environments remain visible for layout. Unclosed math, mismatched environments, invalid groups and extra conditional terminators remain errors in active or unresolved content and after preserved intervals. An unknown `\ifcase` expression, such as `\ifcase\count0`, retains this conservative behavior. A literal number immediately followed by a macro or comment is also unknown, since TeX may continue reading the number. Whitespace, a direct nonnumeric character, or primitive `\relax` confirms the literal endpoint.

`\csname...\endcsname` can construct known conditionals from literal names and known zero-argument macros whose replacement values contain literal characters. This handling follows definition order, grouping and global definitions. Parameterized macros, unknown commands, and replacements requiring execution remain outside that inference; their unresolved structural boundaries retain conservative error behavior. See [document classes and bibliography support](Formatting) for additional built-in signatures.

`syntax.foreach = true` enables recognition of literal `\foreach` headers with variables, optional keys, and an `in` list. It supports nested braced loops, consecutive loop headers, and unbraced bodies through the first top-level semicolon. Braced `\tikz` execution bodies and the text bodies after complete explicit `\node` and `\matrix` headers admit recursive loop recognition. Options, coordinates, and path-local loops retain their layout. Node operations within a path, macro-generated headers, and ambiguous grammar receive conservative handling. A same-source declaration redefining `\foreach` or an explicit `syntax.commands.foreach` entry disables built-in loop recognition. Set `foreach = false` under `[syntax]` to disable it globally. See [loop behavior](Formatting) for examples.

`syntax.environment-arguments.<name>` declares the header arguments for a custom environment or overrides its built-in signature:

| Field | Default for an explicit entry | Behavior |
|---|---|---|
| `optional-arguments` | `false` | Recognize at most one leading `[...]` argument |
| `arguments` | `0` | Number of following mandatory TeX arguments, from 0 to 256 |
| `signature` | unset | Ordered header slots or a complete xparse specification; overrides the legacy field pair |

```toml
[syntax.environment-arguments.custom]
optional-arguments = true
arguments = 2
```

This places the body boundary after `[r]{one}{two}` in `\begin{custom}[r]{one}{two}{body}text`. The `{body}` group remains body content. Mandatory arguments may be brace groups or single TeX tokens. Without an explicit entry, known environments use their built-in ordered signatures; unknown environments recognize at most one leading optional argument and consume no mandatory arguments. To keep an unknown environment's leading `[data]` in its body, use `signature = ""` or set both legacy fields to their defaults in an explicit entry.

Compact environment `signature` values support up to 256 ordered slots: `d` is an optional angle-bracket group such as a Beamer overlay; `m` is a mandatory group or token; `o` is an optional bracket group; `O` is a token-adjacent optional bracket group; `p` is an optional parenthesized group; `g` is an optional brace group; `l` is the exact `\label` token, usually followed by `m` for its argument. `O` can cross comment-suppressed newlines and ignored indentation, while ordinary whitespace separates the option. In compact signatures, `O` carries token-adjacent bracket behavior. Full xparse signatures use `O{default}` for an optional bracket argument with a default value. Missing optional slots leave the cursor in place; a missing `m` stops consumption. A full xparse specification also recognizes `s`, `t<token>`, `e{tokens}`, `E{tokens}{defaults}`, `d<open><close>`, `D<open><close>{default}`, `r/R`, `g/G`, `u{delimiter}`, `v`, `b`, and the `+`, `!`, and `>{processor}` modifiers. Xparse specifications accept whitespace and comments, with at most nine argument values. The formatter identifies argument boundaries and retains defaults and processors; it does not execute processor code. Nonzero legacy `arguments` or `optional-arguments = true` alongside a signature produce a configuration error.

```toml
[syntax.environment-arguments.panel]
signature = 's O{t} D(){auto} m'
```

```toml
[syntax.environment-arguments.mytable]
signature = 'mom'
```

This recognizes all of `{\linewidth}[t]{lX}` as the header of `\begin{mytable}{\linewidth}[t]{lX}`, leaving the following first body group intact. `signature = 'omm'` is equivalent to the legacy example above.

Literal ordinary tcolorbox factories infer a fixed header signature from the current source. The CLI also supplies declarations from supported project inputs, as described below. `\newtcolorbox` and `\renewtcolorbox` support a literal name, an optional numeric parameter count from 0 to 9, and an optional default for the first parameter. `\NewTColorBox`, `\RenewTColorBox`, `\DeclareTColorBox`, and `\ProvideTColorBox` support fixed `m` parameters with at most one leading `o` or `O{default}`. Comments, protected source, macro bodies, and recognized redefinitions constrain inference. An explicit `syntax.environment-arguments` entry takes precedence.

For `\newtcolorbox{custombox}[2][]{title={#1},label={#2}}`, the inferred invocation has one optional parameter followed by one mandatory parameter. Its explicit equivalent is:

```toml
[syntax.environment-arguments.custombox]
optional-arguments = true
arguments = 1
```

Macro-generated names, computed parameter counts, and signatures requiring TeX execution need explicit syntax or protection settings. Literal xparse defaults and processors retain their complete tokens and argument boundaries.

`syntax.infer-list-environments = true` recognizes literal list wrappers declared with `\newenvironment`, xparse environment commands, and enumitem's `\newlist`. Wrappers can refer to other recognized wrappers. A wrapper must open a list at the outer level of its begin body and close it in its end body. Lists inside footnotes and completed nested lists do not classify the surrounding environment as a list. Comments and configured literal regions are excluded. The library analyzes the current source. The CLI uses explicit root and input context when available. For a file with neither a root marker nor a supported literal input, it also reads sibling `.tex`, `.sty`, and `.cls` list declarations, respecting directory ignore rules. This sibling fallback applies to file and named-stdin formatting. Dynamically generated definitions require explicit `syntax.list-environments` entries. Set `infer-list-environments = false` to use only configured list names.

`syntax.command-pattern`, `environment-pattern`, `condition-pattern`, and `item-pattern` constrain or extend structural name matching. `syntax.argument-between` defines accepted separators between command arguments. `[[syntax.special]]` declares `name`, `begin`, `end`, `middle`, `regex`, `nested`, and `verbatim` for custom delimited blocks.

`middle` accepts one delimiter string or an array, using the rule's literal or regex matching. Branch markers return to the matching special block's indentation level, and their following bodies keep that block's extra indentation. Nested blocks, argument contents, math, comments, and protected regions constrain branch ownership. Empty delimiters, begin/end duplicates, and zero-width matches produce errors. Overlapping middle matches at the same position choose the longest match.

```toml
[[syntax.special]]
name = 'algorithmIf'
begin = '\If'
end = '\EndIf'
middle = ['\ElsIf', '\Else']
nested = true
```

Named `breaks.rules.algorithmIf` settings reuse `else-begin` and `else-after` for these branches. `else-after` follows the complete configured middle match; a pattern covering only `\ElsIf` ends before its argument. Use a complete branch-header pattern when applying this boundary, for example `'\\ElsIf\{[^{}]*\}'` for a simple condition without nested braces.

`protect.environments`, `protect.commands`, `protect.no-indent-blocks`, and `protect.file-contents` replace the corresponding built-in name arrays. Boolean `protect.expl3`, `dynamic-catcodes`, `macro-bodies`, and `inactive-branches` control conservative source protection. They default to enabled.

Format controls use `% tift: off`, `% tift: on`, and `% tift: skip`, with ASCII spaces after the percent sign and colon. `off` preserves through its matching `on`, including both marker lines; nested controls are supported. A standalone `skip` preserves itself and the next physical line, while a trailing `skip` preserves its current line. Verbatim content, macro bodies, and Bib values retain their data role. A Bib field selected by `skip` preserves its complete entry. Compact spellings are ordinary comments. Explicit replacement rules using `scope = "all"` retain their configured whole-source behavior. See [package support](Supported-Syntax) for additional command and environment signatures.

Built-in raw environments include moreverb's `verbatimwrite`, `verbatimtab`, `boxedverbatim` and continuation listings, plus tcolorbox's listing, temporary-write, external-source, and documentation code environments. Literal `\newminted`, `\lstnewenvironment`, `\NewTCBListing`, and comment factory declarations identify additional raw environments and preserve the declaration itself. The inferred names follow source declarations; recognized command or environment redefinitions and explicit `syntax.commands` entries constrain inference.

Inline raw commands include `\verb`, `\lstinline`, `\mintinline`, `\mint`, and `\tcboxverb`. Minted commands keep their language argument before the code delimiter; supported code braces and delimiters preserve the original content. Comment terminators begin at the start of their physical line; trailing ASCII spaces discarded by TeX's input reader are accepted.

The name `listing` uses package context: an explicit top-level moreverb load selects raw code, while minted's ordinary listing float receives normal formatting. Comments, configured raw regions, disabled regions, and recognized inactive branches stay outside package-load inference. Conflicting package loads preserve the ambiguous body conservatively. Source-hidden loads and dynamic declarations require explicit protection configuration. Comment factory bodies retain their physical line layout and use the package's complete-line terminator.

Physics `\mqty`, `\matrixquantity`, `\smqty`, and `\smallmatrixquantity` preserve their brace, bracket, parenthesis, and vertical-bar arguments, including stars. The internal `&` and `\\` tokens belong to those arguments, so outer alignment measures the command as an opaque cell fragment. Local redefinitions and explicit command syntax control this built-in recognition.

With dynamic catcode protection enabled, literal environment definitions in the current file are checked for tokenization-changing commands. Protection propagates through environment wrappers and command dependencies; affected begin/body/end invocation ranges remain intact. The CLI also supplies supported protection declarations from explicit root and input context. External declarations outside that context require explicit protection configuration. See [dynamic environment behavior](Formatting) for the `%` catcode example and the source-level analysis.

`protect.format-macro-bodies = true` applies indentation to structurally safe LaTeX macro definitions. Their contents remain protected from structural boundary changes, prose wrapping, table alignment, replacements, and trailing-space removal. Environment definitions are checked across their paired begin/end bodies. Structurally safe `\def`, `\gdef`, `\edef`, and `\xdef` bodies also receive indentation. Expansion control, dynamic catcodes, nested definitions, and conditional branches crossing definition bodies retain their source. Set `format-macro-bodies = false` to preserve every macro body exactly. Literal catcode assignments remain protected until their standard categories are restored or their local scope ends. Queries and assignments retaining the standard category preserve only the assignment itself. Nested scopes retain their outer category state. Conditions encountered during recovery, unknown assignments, and changes to escape or grouping syntax keep conservative protection through the reliable enclosing scope or EOF. Expl3 regions end at executed `\ExplSyntaxOff` boundaries; occurrences in comments and macro bodies do not end the region.

## Project declaration context

The CLI can read project declarations while formatting one source file. A standalone magic comment selects an explicit root:

```tex
% !TEX root = ../main.tex
```

The path is relative to the file containing the comment. Leading whitespace, case variations in `TEX root`, and single or double quotes around the path are accepted. Markers inside protected literal content are ignored. Root files can carry another root marker; the CLI follows that explicit chain. A marker referring to its own file ends the chain. An unreadable root, an empty root path, or a cycle between distinct roots is an error. Files without a marker use their own input context; the CLI does not search the workspace for a parent main file.

The selected root's `\input{path}` and `\include{path}` references are followed recursively when their paths are literal or composed from known zero-argument macros. For example, `\def\chapterfile{chapters/one}` allows both `\input{\chapterfile}` and bare `\input\chapterfile`. Definitions are resolved in source order, including declarations from earlier local imports; `edef/xdef` retain frozen values, and `let` copies the current replacement. Local class and package imports are also followed through `\documentclass`, `\LoadClass`, `\LoadClassWithOptions`, `\usepackage`, `\RequirePackage`, and `\RequirePackageWithOptions`. Their declarations contribute custom command and environment signatures. A local `.cls` takes precedence over built-in adaptation for the same class name. Bare `\input path` is also supported. Omitted `.tex` extensions are accepted. Paths are tried relative to the root's directory first, then the directory containing the input command. Missing input files are skipped. Parameterized or execution-dependent path macros, uncertain bindings, TeX search-path lookup and `\import` need explicit syntax or protection settings. Input cycles and literal expansion are bounded; see [declaration analysis](Project-Context) for limits and constructed command names.

`syntax.input-ignore-extensions` controls resource suffixes skipped by declaration
imports through `input/include`. Its default is `svg`, `png`, `pdf`, `jpg`,
`jpeg`, `gif`, `bmp`, `webp`, `tif`, `tiff`, `eps`, `ps`, `mp4`, `m4v`, `mov`,
`avi`, `webm`, `mp3`, `wav`, `ogg`. Matching ignores ASCII case and accepts an
optional leading dot in configured entries. The last extension is compared;
`.pdf_tex`, `.eps_tex`, `.pstex_t`, unknown extensions and extensionless paths
remain eligible. An explicit array replaces the defaults; `[]` disables suffix
filtering for projects storing TeX macros in files such as `macros.pdf`. The
setting leaves input commands in the formatted text and does not filter local
class or package imports.

Explicit `syntax.commands` and `syntax.environment-arguments` entries let
ordinary LaTeX/xparse declarations skip redundant argument count and signature
interpretation for the same name. Completion, scope, condition, loader binding,
list inheritance and literal protection remain checked when needed. Configuring
one name does not suppress other declarations in its source file.

For example:

```text
project/
├── main.tex
├── config/set.tex
└── chapters/chap1.tex
```

```tex
% main.tex
\documentclass{article}
\input{config/set}
\begin{document}
\input{chapters/chap1}
\end{document}
```

`tift --print main.tex` reads declarations in `config/set.tex` and formats the main source using that context. The printed result retains its `\input` commands. For `tift --print chapters/chap1.tex`, put `% !TEX root = ../main.tex` in the chapter to obtain the same project declarations. Standard input supports the marker with `--stdin-filename chapters/chap1.tex`, including a filename that has not yet been saved. A directory invocation such as `tift .` applies this resolution to each selected file and reuses input snapshots within the invocation.

The declaration context recognizes fixed literal LaTeX environment and command definitions, supported xparse signatures, list wrappers, ordinary tcolorbox factories, and supported raw-content factories and wrappers. Environment and command declarations retain complete xparse signatures, including interleaved optional/defaulted slots, token switches, embellishments, custom delimiters, and argument processors. Named alignment settings are still needed to select an otherwise unknown custom table environment for column alignment. Explicit TOML syntax and protection settings take precedence.

For a chapter included by the root, declarations are collected in input order through that chapter's first occurrence, excluding later siblings. A chapter not yet included by its explicit root receives the root's preamble context and its own declarations. Root/input contexts use the actual references, so unrelated sibling settings files do not contribute declarations. The context summarizes declarations for the target file; declaration changes at individual invocation positions and arbitrary TeX execution remain outside this analysis. Conditional, grouped, and non-document-environment inputs are skipped conservatively. Cross-file comment-factory terminator semantics currently require explicit protection configuration.

Only selected formatting targets are written or printed; reading a dependency for declarations does not add it as a formatting target. TOML discovery and file overrides continue to use the target file's path. The library does not read files; CLI project context is supplied as inferred configuration. `--replacement-only` and BibTeX inputs skip declaration context.

## Whitespace

`whitespace.remove-trailing = true` removes unprotected line-end whitespace. `before-processing = false` selects cleanup after layout. `preserve-blank-lines = true` retains blank lines up to the default `max-blank-lines = 2`. Set this value to any nonnegative maximum. Literal source, macro bodies, multiline Bib values and Bib comment entries keep their protected contents. `line-ending` accepts `"preserve"`, `"lf"`, and `"crlf"`; `final-newline` accepts `"preserve"`, `"always"`, and `"never"`.

## Replacements

Each `[[replacements]]` table requires a nonempty `find`. `replace` supplies its replacement. The defaults are literal matching, the `"before"` stage, `"unprotected"` scope, and `enabled = true`.

```toml
[[replacements]]
find = '(?<=\b)colour\b'
replace = 'color'
regex = true
engine = "enhanced"
stage = "after"
scope = "unprotected"
count = 1
backtrack-limit = 1000000
```

`engine = "rust"` uses linear regular expressions; `"enhanced"` supports lookaround and backreferences. Regex replacements support capture interpolation such as `$1` and `${name}`. `count` limits matches across the whole file, including separate unprotected spans. Rules execute in declaration order within their stage. `scope = "all"` permits changes to literal/protected contents. `replacement-only = true` or `--replacement-only` runs configured replacement stages without other layout passes.

## BibTeX and BibLaTeX

These options belong under `[bibtex]` and apply to `.bib` files.

| Field | Default | Behavior |
|---|---|---|
| `normalize` | `true` | Normalize assignment layout |
| `name-case` | `"preserve"` | Preserve entry-type and field-name case; select `"lower"` or `"upper"` for uniform case |
| `align-fields` | `true` | Align assignments when normalization is enabled |
| `value-delimiters` | `"preserve"` | Preserve values, or use `"braces"` / `"quotes"` for safe literal pieces |
| `field-order` | `[]` | Preserve order, use `["canonical"]` / `["alphabetical"]`, or list preferred fields |
| `sort` | `false` | Enable stable sorting within movable entry groups |
| `sort-by` | `["key"]` | Successive citation-key, entry-type, or field criteria; `-` prefixes descending criteria |
| `case-sensitive` | `false` | Case-sensitive sorting when enabled |

Assignment normalization is enabled by default; set `normalize = false` to use uniform assignment spacing. Entry types and regular field names preserve source case by default. `name-case = "lower"` or `"upper"` selects uniform case independently of assignment layout. Field ordering and entry sorting can be configured independently and default to preserving authored order. Citation keys, macro references, value expressions, raw fields, and protected spans retain their data. Definitions and outside comments form sorting boundaries; linked bibliography entries retain their source-relative precedence. See [BibTeX and BibLaTeX](BibTeX) for complete examples and comparison rules.

Data cleanup is selected under `[bibtex.cleanup]`. Its `enabled` switch and each operation default to `false`; `max-authors` is unset. Select the operations the project needs:

```toml
[bibtex.cleanup]
enabled = true
months = true
remove-duplicate-fields = true
remove-empty-fields = true
```

Other operations are `sentence-case-titles`, `merge-duplicate-entries`, `normalize-urls`, `escape-special-characters`, `generate-missing-keys`, and a positive `max-authors` limit. Cleanup preserves conflicting values and existing citation keys. Its [complete option reference](BibTeX#optional-data-cleanup) describes merge boundaries, literal values and conservative diagnostics.

## Code and docstrip layout

| Field | Default | Behavior |
|---|---|---|
| `code.algorithm2e` | `true` | Expand recognized braced algorithm2e control flow and separate statements |
| `code.expl3` | `true` | Format recognized expl3 code regions and definitions |

`.dtx` and `.ins` files select docstrip layout automatically. Documented TeX margins, guards, and installation preambles retain their roles. The code options also apply to eligible regions in ordinary TeX, style, and class files and can each be disabled. Dedicated layout protects formula source while laying out code and uses the configured indentation width. Native named indentation, structural breaks, argument layout, and column-spec alignment currently apply to ordinary TeX stages; the docstrip code layer uses the native code printer. See [code and docstrip formatting](Code-Formatting) for coverage and examples.

## Formula layout

| Field | Default | Behavior |
|---|---|---|
| `math.enabled` | `true` | Enable mathematical layout |
| `math.spacing` | `true` | Normalize horizontal whitespace independently of wrapping |
| `math.wrap` | `"break"` | `"preserve"`, `"single-line"`, or width-based `"break"` |
| `math.columns` | unset | Formula width; omission inherits `wrap.columns` |
| `math.break-priority` | `[]` | Highest-first operator classes: `relation`, `additive`, `multiplicative`, `binary`; omission keeps greedy wrapping |

`math.columns` accepts values from 1 through 1,000,000. `math.break-priority` rejects repeated classes.

The native math stage applies to eligible formulas in ordinary TeX, style, class, `.dtx`, and `.ins` files. Code layout preserves math as complete units while laying out surrounding statements; the separate math pass then controls their internal formatting. DTX documentation formulas are processed before prose wrapping. INS preambles and postambles remain literal generated-file data. Protected regions retain their contents.

Set `[math] enabled = false` to retain formula-internal spacing and line breaks. Ordinary TeX indentation and outer equation alignment continue to follow their own settings. Long single-formula displays use `math.columns`, falling back to `wrap.columns`; display indentation follows the corresponding named indentation rules. Known binary and relation gaps are normalized and script attachment gaps are tightened. Slash and paired-parenthesis interior gaps are preserved when both sides contain a space; one-sided gaps are removed. Repeated same-line math spaces collapse to one. Script control-word operator gaps retain their authored spacing. See [formula formatting](Math-Formatting) for examples.

## Tree source layout

`[trees] mode = "expand"` indents child bracket subtrees in `forest` environments and qtree's `\Tree`. The default mode is `"preserve"`; `"auto"` expands trees that exceed `trees.columns`, or `wrap.columns` when omitted. `forest = false` or `qtree = false` disables one dialect. Labels, options and braced contents remain literal; ambiguous optional syntax and deferred `\node`/`\edge` declarations retain their source layout. Leaves remain compact.

`trees.columns` accepts values from 1 through 1,000,000. See the [forest and qtree examples](Formatting-Examples#forest-and-qtree-trees) for input and output.

## File selection and inheritance

`files.exclude` is an array of Git ignore patterns. `files.threads` supplies a positive worker count. `extend = "path/to/base.toml"` imports one configuration. `[[overrides]]` requires a nonempty array of `files` patterns and accepts regular configuration tables. Nested overrides and `extend` inside an override are rejected.

Array replacement applies to exclusions, override arrays, command lists, abbreviations, and other array fields. Inherited file patterns retain the directory of the file declaring the array. Patterns outside that directory have no match; the global configuration still applies to explicitly selected inputs there.

The CLI starts at each input file's directory and searches upward for the nearest project `tift.toml`, stopping at a Git root or filesystem root. Put `tift.toml` beside the project's main source to share settings with files in subdirectories. A nearer subdirectory configuration takes over for files beneath it; use `extend` when it should inherit the parent configuration.

When no project configuration is found, the CLI uses a personal configuration. A nonempty `TIFT_CONFIG_HOME` selects `TIFT_CONFIG_HOME/tift.toml`. Otherwise, the platform paths are:

| Platform | Personal configuration path |
|---|---|
| Windows | `%APPDATA%/tift/tift.toml`; falls back to `%USERPROFILE%/AppData/Roaming/tift/tift.toml` when `APPDATA` is unavailable |
| Linux and other Unix | `$XDG_CONFIG_HOME/tift/tift.toml` when set; otherwise `$HOME/.config/tift/tift.toml` |
| macOS | `$XDG_CONFIG_HOME/tift/tift.toml` when set; otherwise `$HOME/Library/Application Support/tift/tift.toml` |

An empty `TIFT_CONFIG_HOME` disables personal discovery. `--config path/to/custom.toml` selects that file; `--no-config` uses built-in defaults and command-line settings. Project configuration and these two options bypass the personal fallback. The selected project configuration can use `extend` to explicitly import a personal or shared configuration. Personal configuration is useful for general formatting defaults; its exclusion and override patterns still address descendants of the directory declaring them.

Settings apply in this order, with later values taking precedence:

1. Built-in defaults.
2. The selected configuration's `extend` chain, from the oldest ancestor to the nearest base.
3. The selected configuration's own fields.
4. Matching `[[overrides]]` entries, in declaration order.
5. Command-line options.

For example, this project configuration keeps prose line breaks and applies a 100-column budget to selected command arguments:

```toml
[indent]
width = 4

[wrap]
mode = "preserve"

[arguments]
mode = "auto"
columns = 100
```

Save it as `tift.toml` in the project directory, then use `tift --diff .` to inspect changes or `tift .` to format the selected files. The same TOML fields are accepted in a personal configuration.

`tift --show-config path/to/paper.tex` prints merged TOML with configuration sources, matching override count, and exclusion status. Virtual paths and `--stdin-filename` are accepted. This view does not read the source; root comments, input declarations, and sibling list inference are applied during actual formatting. `--trace` writes per-file stage changes, byte counts, and durations to stderr. See the README for output paths, backups, completion generation, and exit codes.

`[cache]` controls the CLI's persistent formatting-result files and defaults to
`enabled = true`. Set `enabled = false` or pass `--no-cache` to disable reading and
writing these files. Internal configuration, source-analysis, project-declaration,
indentation-prefix and compiled-syntax reuse stays enabled in either mode.
Persistent result files are managed by the CLI; direct library formatting calls
use their normal in-memory analysis.

```toml
[cache]
enabled = true
directory = ".tift-cache"
```

`directory` is optional. A configuration path is resolved relative to the file
that declares it, including inherited configurations and overrides.
`--cache-dir PATH` takes precedence and resolves a relative path from the CLI's
working directory. `TIFT_CACHE_HOME` selects the default cache directory; an empty
value disables default-directory selection. Explicit configuration and CLI paths
still take precedence. Platform defaults are `%LOCALAPPDATA%/tift/cache` on Windows,
`~/Library/Caches/tift` on macOS, and `$XDG_CACHE_HOME/tift` or `~/.cache/tift` on Linux.

JSON files under `<cache directory>/v2/` group inputs by source directory. Each
input retains its most recent actual input-to-output formatting result. A group
is read once per invocation and written once if its entries changed. The cache
key covers the exact source bytes, effective
configuration, file kind and executable content hash. Each CLI run prepares the
current project declaration context before lookup, so dependency and declaration
changes take effect. A hit reuses the formatting result and diagnostics. Changed
input, settings or program content require formatting again.

`--trace`, `--trace-detail` and `--lines` run the formatter directly. Failed results
are left out of the cache; corrupt records and unavailable or unwritable storage
fall back to formatting. Ordinary summaries include hit and miss counts;
`--quiet` and `--print` retain their existing stderr behavior.

The cache stores observed input-to-output results. When an in-place write changes
the source, the new bytes need one actual formatting pass before they can be
reused. This preserves configured replacements that intentionally change their
input on successive runs. See [ranges and tracing](Cache-and-Tracing) for line
selections and the detailed tracing interface.
