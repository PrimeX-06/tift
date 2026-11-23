# Formatting examples

Use `tift --print input.tex` to inspect formatted source. Place the accompanying settings in `tift.toml`; unspecified settings use the built-in defaults. Bibliography examples use `input.bib`.

The [README](https://github.com/PrimeX-06/tift) introduces the common formats. This gallery adds custom rules and less frequently used options. See [configuration setup](Configuration-Guide) for file locations and inheritance, and the [configuration reference](Configuration) for every field.

| Example | What it demonstrates |
|---|---|
| [Nested indentation and list rules](#nested-indentation-and-list-rules) | Nested blocks, list items and continuations, named indentation rules |
| [Tables, floats, and Unicode alignment](#tables-floats-and-unicode-alignment) | Column alignment, Unicode width, row breaks, table rules, captions |
| [Left, center, and right columns](#left-center-and-right-columns) | Per-column `l/c/r` source justification |
| [Aligned equations and matrices](#aligned-equations-and-matrices) | Equation columns and nested matrix alignment |
| [TikZ paths and nested loops](#tikz-paths-and-nested-loops) | Path boundaries, continuation indentation, nested foreach scopes and data lists |
| [Forest and qtree trees](#forest-and-qtree-trees) | Bracket subtrees, labels, and tree width settings |
| [Text wrapping, comments, and sentences](#text-wrapping-comments-and-sentences) | Prose and comment wrapping, sentence lines, text arguments, atomic math |
| [Nested key/value and image options](#nested-keyvalue-and-image-options) | Recursive argument layout, comma boundaries, image options and filenames |
| [Source protection, macros, and conditionals](#source-protection-macros-and-conditionals) | Safe macro indentation, verbatim data, off/on/skip controls, inactive branches |
| [Custom headers, alignment, and branch blocks](#custom-headers-alignment-and-branch-blocks) | Source declaration inference, ordered signatures, custom table and branch rules |
| [BibTeX fields and values](#bibtex-fields-and-values) | Field indentation and spacing, stable order, literal values |
| [Literal and regex replacements](#literal-and-regex-replacements) | Ordered replacements, captures, protection scopes |
| [Formatting selected lines](#formatting-selected-lines) | Full-document context with exact unselected lines |
| [Whitespace and line endings](#whitespace-and-line-endings) | Trailing spaces, blank-line limits, LF/CRLF, final newline |

See [supported syntax](https://github.com/PrimeX-06/tift/wiki/Supported-Syntax) for additional package forms and protection limits.

## Nested indentation and list rules

The global indentation unit is four spaces. A named quote rule uses two spaces for that wrapper, while nested lists and item continuations keep their own structural levels.

**tift.toml**

```toml
[indent.rules.quote]
width = 2
```

**Input**

```tex
\begin{document}
\begin{quote}
\begin{itemize}
\item First observation.
Its explanation continues here.
\item Second observation.
\end{itemize}
\end{quote}
\end{document}
```

**Output**

```tex
\begin{document}
\begin{quote}
  \begin{itemize}
      \item First observation.
          Its explanation continues here.
      \item Second observation.
  \end{itemize}
\end{quote}
\end{document}
```

## Tables, floats, and Unicode alignment

Display-width alignment accounts for wide Unicode text. Float indentation, complete booktabs rules, row boundaries, and the following caption share the same table structure. Built-in package signatures also cover tables such as tabularray and NiceTabular.

**tift.toml**

```toml
[alignment]
width = "display"
```

**Input**

```tex
\begin{table}
\begin{tabular}{ll}\toprule 中文&Value\\\midrule Longer name&42\\\bottomrule\end{tabular}\caption{Measurements}
\end{table}
```

**Output**

```tex
\begin{table}
    \begin{tabular}{ll}
        \toprule
        中文        & Value \\
        \midrule
        Longer name & 42 \\
        \bottomrule
    \end{tabular}
    \caption{Measurements}
\end{table}
```

## Left, center, and right columns

Each source column follows its own specification. The `Item` column uses `l`, the `Label` column uses `c`, and the `Count` column uses `r`. Cell widths determine the spaces before and after each cell.

**Input**

```tex
\begin{tabular}{lcr}
Item&Label&Count\\
Long item&B&1234\\
A&Wide label&7\\
\end{tabular}
```

**Output**

```tex
\begin{tabular}{lcr}
    Item      &   Label    & Count \\
    Long item &     B      &  1234 \\
    A         & Wide label &     7 \\
\end{tabular}
```

## Aligned equations and matrices

Math alignment applies to complete outer cells; nested matrix content uses its own alignment rules.

**Input**

```tex
\begin{align}
x&=1\\longer&=a+b\\
\end{align}
\begin{pmatrix}
1&22\\333&4
\end{pmatrix}
```

**Output**

```tex
\begin{align}
    x      & = 1     \\
    longer & = a + b \\
\end{align}
\begin{pmatrix}
    1   & 22 \\
    333 & 4
\end{pmatrix}
```

## TikZ paths and nested loops

Complete outer semicolons separate paths and nodes, and multiline paths receive continuation indentation. Each literal foreach loop adds a level; the following path returns to the picture level. Options, coordinates, and node text retain their ownership.

**Input**

```tex
\begin{tikzpicture}
\draw[thick] (0,0)
-- (1,1);\node at (1,1) {Result};
\foreach \x in {0,1}
\foreach \y in {0,1}
{
\draw (\x,\y) circle (2pt);
}
\draw (0,0) circle (1pt);
\end{tikzpicture}
```

**Output**

```tex
\begin{tikzpicture}
    \draw[thick] (0,0)
        -- (1,1);%
    \node at (1,1) {Result};
    \foreach \x in {0,1}
        \foreach \y in {0,1}
        {
            \draw (\x,\y) circle (2pt);
        }
    \draw (0,0) circle (1pt);
\end{tikzpicture}
```

### Foreach data lists

Set `arguments.foreach-lists` to `"expand"` to put each literal record on its own line. The default `"preserve"` keeps the list's authored layout. `"auto"` expands lists exceeding `arguments.columns`, falling back to `wrap.columns`.

**tift.toml**

```toml
[arguments]
foreach-lists = "expand"
```

**Input**

```tex
\begin{tikzpicture}
\foreach \point in {(0,1),(2,3),(4,5)} {\draw \point circle (1pt);}
\end{tikzpicture}
```

**Output**

```tex
\begin{tikzpicture}
    \foreach \point in {%
        (0,1),%
        (2,3),%
        (4,5)%
    } {%
        \draw \point circle (1pt);%
    }
\end{tikzpicture}
```

Coordinates, slash-separated records, and braced values remain together. Compact ranges such as `{1,...,100}` and macro-supplied lists retain their layout. Inserted percent comments preserve TeX spacing at new line boundaries.

## Forest and qtree trees

Set `trees.mode` to `"expand"` to indent literal child subtrees in `forest` environments and qtree's `\Tree`. The default `"preserve"` keeps the authored tree layout.

**tift.toml**

```toml
[trees]
mode = "expand"
```

**Input**

```tex
\begin{forest}
[root [left [leaf]] [right]]
\end{forest}
```

**Output**

```tex
\begin{forest}
    [root
        [left
            [leaf]%
        ]
        [right]%
    ]
\end{forest}
```

`mode = "auto"` expands trees exceeding `trees.columns`, or `wrap.columns` when the tree width is omitted. `forest = false` or `qtree = false` disables the corresponding dialect. Labels, options, braced contents, and leaves retain their contents. Ambiguous syntax and deferred `\node` or `\edge` declarations retain their source layout.

## Text wrapping, comments, and sentences

One width budget covers prose, comments, and known ordinary-text arguments. `wrap.mode = "wrap"` retains existing source boundaries, while `"rewrap"` joins old lines and wraps the paragraph. Sentence splitting starts new sentences on source lines, and inline math stays atomic. Use `"unwrap"` to join eligible prose without applying a width limit.

**tift.toml**

```toml
[wrap]
mode = "wrap"
columns = 48
comments = true

[sentences]
enabled = true
preserve-line-breaks = true
```

**Input**

```tex
\begin{quote}
This sentence describes a long observation while keeping $x+y=z$ together in the source. A second sentence starts here.
% This long comment explains the measurements and the assumptions used by the model.
An existing short line stays here.
\end{quote}
\caption{A long caption describes the measurements and explains the relationship between the variables.}
```

**Output**

```tex
\begin{quote}
    This sentence describes a long observation
    while keeping $x + y = z$ together in the
    source.
    A second sentence starts here.
    % This long comment explains the
    % measurements and the assumptions used by
    % the model.
    An existing short line stays here.
\end{quote}
\caption{A long caption describes the
    measurements and explains the relationship
    between the variables.}
```

## Nested key/value and image options

Nested key-setting commands expand from the inside out. `\pgfkeys` splits entries only at top-level commas, so commas and equals signs inside grouped values, math or literal commands stay with their value. Handler definitions such as `/.code` and `/.style` keep their code bodies intact while the outer key list is formatted. Active wrapping supplies a width for image options, while filenames and grouped values retain their data. Empty percent comments preserve tight TeX spacing at inserted boundaries.

**tift.toml**

```toml
[wrap]
mode = "wrap"
columns = 55

[arguments]
mode = "expand"
```

**Input**

```tex
\pgfkeys{outer={\tikzset{alpha={one}, beta={two}, gamma={three}}}, next={value}}
\includegraphics[width=0.8\linewidth,height=4cm,keepaspectratio,angle=0]{figures/measurement-plot.pdf}
```

**Output**

```tex
\pgfkeys{%
    outer={\tikzset{%
            alpha={one},
            beta={two},
            gamma={three}%
        }},
    next={value}%
}
\includegraphics[%
    width=0.8\linewidth,%
    height=4cm,%
    keepaspectratio,%
    angle=0%
]{figures/measurement-plot.pdf}
```

## csquotes quotation headers

csquotes display environments keep citation metadata in the opening header and leave the quotation body available to normal prose formatting. Foreign-language forms add a leading language argument.

**Input**

```tex
\begin{displaycquote}[see][12]{smith2024}[.]A quoted sentence with its own paragraph.
The continuation remains part of the quotation body.
\end{displaycquote}

\begin{foreigndisplayquote}{french}[Author][.]
Texte cite avec accents et ponctuation.
\end{foreigndisplayquote}
```

**Output**

```tex
\begin{displaycquote}[see][12]{smith2024}[.]
    A quoted sentence with its own paragraph.
    The continuation remains part of the quotation body.
\end{displaycquote}

\begin{foreigndisplayquote}{french}[Author][.]
    Texte cite avec accents et ponctuation.
\end{foreigndisplayquote}
```

### Aligning key/value assignments

Enable `arguments.align-equals` to align assignment operators in selected multiline option lists. The default is `false`.

**tift.toml**

```toml
[arguments]
align-equals = true
```

**Input**

```tex
\tikzset{
a=1,
long key={x=y,z}
}
```

**Output**

```tex
\tikzset{
    a        = 1,
    long key = {x=y,z}
}
```

Nested braced values retain their contents. Single-line lists stay compact unless active argument expansion rules require a break. Named `arguments.rules.<command>.align-equals` settings override the global switch. See [argument settings](Configuration#command-argument-layout) for command selection and width rules.

## Source protection, macros, and conditionals

Safe macro definitions receive indentation with body text and line layout protected. Verbatim data, off/on regions, skipped physical lines, and known inactive conditional source retain their bytes. Ordinary active source remains available for formatting.

**Input**

```tex
\newcommand{\message}[1]{
\begin{quote}
Hello, #1.
\end{quote}
}
\begin{quote}
\begin{verbatim}
  literal { braces }  &  data
\end{verbatim}
% tift: off
 unaligned   source
% tift: on
 keep this line   % tift: skip
\iffalse
Unbalanced { and $ data stays here.
\fi
Ordinary text with \verb|x & { literal }|.
\end{quote}
```

**Output**

```tex
\newcommand{\message}[1]{
    \begin{quote}
        Hello, #1.
    \end{quote}
}
\begin{quote}
\begin{verbatim}
  literal { braces }  &  data
\end{verbatim}
% tift: off
 unaligned   source
% tift: on
 keep this line   % tift: skip
    \iffalse
Unbalanced { and $ data stays here.
    \fi
    Ordinary text with \verb|x & { literal }|.
\end{quote}
```

## Custom headers, alignment, and branch blocks

The source-defined note environment supplies its own header. An explicit mom signature describes the custom table's mandatory/optional/mandatory arguments, and a named alignment rule selects its rows. Special block markers supply branch indentation inside the same document.

**tift.toml**

```toml
[syntax.environment-arguments.mytable]
signature = "mom"

[alignment.rules.mytable]
enabled = true

[[syntax.special]]
name = "algorithmIf"
begin = '\If'
end = '\EndIf'
middle = ['\ElsIf', '\Else']
nested = true
```

**Input**

```tex
\NewDocumentEnvironment{note}{m O{Info}}{}{}
\begin{note}{Report}[Details]
\If{ready}
\begin{mytable}{Measurements}[t]{ll}
A&B\\Longer&42\\
\end{mytable}
\Else
\State Wait.
\EndIf
\end{note}
```

**Output**

```tex
\NewDocumentEnvironment{note}{m O{Info}}{}{}
\begin{note}{Report}[Details]
    \If{ready}
        \begin{mytable}{Measurements}[t]{ll}
            A      & B  \\%
            Longer & 42 \\
        \end{mytable}
    \Else
        \State Wait.
    \EndIf
\end{note}
```

## BibTeX fields and values

Default bibliography formatting preserves entry-type and field-name case and aligns assignments while preserving entry/field order and literal value contents. `[bibtex] name-case = "lower"` or `"upper"` selects uniform name case. Set `[bibtex] normalize = false` to use uniform assignment spacing.

**Input**

```bibtex
@article{sample,title={A  {Nested} Title},author="Doe, Jane",year=2026}
```

**Output**

```bibtex
@article{sample,
    title  = {A  {Nested} Title},
    author = "Doe, Jane",
    year   = 2026,
}
```

## Literal and regex replacements

Replacement rules run in declaration order and support bounded regex capture interpolation. Unprotected scope keeps literal regions outside replacement edits.

**tift.toml**

```toml
[[replacements]]
find = "colour"
replace = "color"
scope = "unprotected"

[[replacements]]
find = 'v([0-9]+)'
replace = 'version $1'
regex = true
scope = "unprotected"
```

**Input**

```tex
The colour is red. Release v1.
\verb|colour v1|
```

**Output**

```tex
The color is red. Release version 1.
\verb|colour v1|
```

## Formatting selected lines

The CLI can format one-based selections using full-document structure. Unselected physical lines retain their source bytes.

**Command**

```sh
tift --print input.tex --lines 2,4
```

**Input**

```tex
\begin{quote}
First line.
Second line.
Third line.
\end{quote}
```

**Output**

```tex
\begin{quote}
    First line.
Second line.
    Third line.
\end{quote}
```

## Whitespace and line endings

Escaped strings make trailing spaces, CRLF endings, blank lines, and the final newline visible. This configuration removes trailing whitespace, limits blank runs, and selects LF with a final newline.

**tift.toml**

```toml
[whitespace]
max-blank-lines = 1
line-ending = "lf"
final-newline = "always"
```

**Input**

```json
"Alpha.  \r\n\n\n\nBeta.\t"
```

**Output**

```json
"Alpha.\n\nBeta.\n"
```
