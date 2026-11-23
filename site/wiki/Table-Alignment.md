# Table column alignment

Table source follows recognized column specifications by default:

```toml
[alignment]
justification = "columns"
```

`l`, `c`, and `r` select left, center, and right justification for each source cell. Use `"left"`, `"center"`, or `"right"` for uniform justification, or set a named environment rule:

```toml
[alignment.rules.tblr]
justification = "columns"

[alignment.rules.tabular]
justification = "left"
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

## Column specification layout

Use `alignment.column-spec` to control the source layout of a table's column specification. The default `"preserve"` keeps its authored layout. `"auto"` expands a recognized preamble when it exceeds `wrap.columns`; `"expand"` puts each logical column on its own line.

**tift.toml**

```toml
[alignment]
column-spec = "expand"
```

**Input**

```tex
\begin{tabular}{@{}lcr@{}}
a&b&c\\
\end{tabular}
```

**Output**

```tex
\begin{tabular}{%
    @{}l%
    c%
    r@{}%
}
    a & b & c \\
\end{tabular}
```

Modifiers remain attached to their logical columns. Dynamic or unsupported specifications retain their layout. A named `alignment.rules.<environment>.column-spec` setting overrides the global mode. Source cell justification continues to follow `alignment.justification`.

## Recognized specifications

Static specifications cover 54 formatting environments, including standard `tabular` and `array`, long and width-constrained tables, `tabulary`, `tabu`/`longtabu`, NiceTabular and NiceArray families, subarrays, and tabularray tables and mathematical wrappers.

Specifications directly recognize `l/c/r/X/Q/L/C/R/m/b/p/S/s` across ordinary tables, NiceTabular, and tabularray. `l/L` select left, `c/C` center, and `r/R` right justification. `X/Q` and `p/m/b` width columns default to left; `S/s` numeric columns default to center. `L/C/R` may include an opaque width argument. Repetitions (`*{n}{...}`), rules, and `>`, `<`, `@`, `!` modifiers retain their source spelling.

Complete literal `\newcolumntype` and `\NewColumnType` definitions override these defaults: `\centering`, `\raggedright`, and `\raggedleft` select center, left, and right. Definitions can use mandatory width parameters, `Q[halign=...]`, or `X[...]`. For example, `\newcolumntype{S}{l}` selects left justification for `S` in that source. NiceTabular width columns accept horizontal options such as `p[l]{2cm}`, `m[c]{2cm}`, and `b[r]{2cm}`. A `\multicolumn` cell keeps its logical span and uses a recognized single-column override for its justification.

Widths and placement arguments belong to the complete table header. For tabu, recognized literal forms include `to .8\linewidth [t]{lcr}`, `spread 1cm {lcr}`, and `to \dimexpr\linewidth-2em\relax {lcr}`.

## Tabularray rules

Inner keys apply in source order: `colspec`, `rowspec`, `columns`, `rows`, `cells`, and selected row, column, or cell rules. Later horizontal settings override earlier ones.

```tex
\begin{tblr}{
    colspec={lcr},
    rows={c},
    column{3}={r},
    cell{1}{2}={l}
}
    a & b & c \\
    long & wide & 100 \\
\end{tblr}
```

The first row uses center, left, right justification. Subsequent rows use center, center, right. Standalone rules and comments retain their positions and do not count as data rows.

Selectors include numeric lists, ranges, all rows/columns (`-`), trailing indices `U` through `Z`, `Z[n]`, odd/even selectors with optional starting indices, and `every[step]{start}{end}`. Negative `every` endpoints count backward from the row or column total. Paired cell coordinates such as `cell{{1}{2},{3}{4}}` are supported.

`Q/X` options accept literal `l/c/r/j` and `halign`. Source justification for `j` is left. `S/s` numeric columns use centered source alignment directly; package and library loading declarations retain their bytes. The six `+matrix` wrappers default to center, while `+cases` defaults to left.

## Spans and protection

Top-level `\SetCell`, `\SetRow`, `\SetColumn`, `\SetRows`, `\SetColumns`, and `\SetCells` apply after the header in source order. Singular row and column commands select their current logical row or column; plural commands select all corresponding entries. Later commands can override earlier cells. Grouped and nested environment settings retain their local scope. A spanning anchor and its empty `&` placeholders are measured against the combined width of their columns. The following real column boundary aligns with ordinary rows; internal placeholder delimiters move with the anchor. Every original `&` is retained. Cells covered by a row span retain their placeholders and do not contribute to measured column widths.

```tex
\begin{tblr}{colspec={lcr}}
    \SetCell[c=2]{r} span && x \\
    first & second & last \\
\end{tblr}
```

Subsequent `cell`, `cells`, `\SetCell`, or `\SetCells` calls reset omitted span values to 1. Protected content, explicit syntax rules, disabled environments, child-environment settings, CRLF, and selected-line boundaries remain effective.

Unknown column arity, dynamic specifications or selectors, unknown horizontal styles, large/deep repetitions, and explicit custom environment signatures fall back to uniform left alignment. Custom column definitions in other files are not expanded. Source redefinitions and `SetTblrInner`/`SetTblrDefault` disable the affected tabularray inference. Invalid local `SetCell` spans retain the affected source row.

`ocgtabular` has column-slot metadata; its listings data body keeps its default literal protection. Ordinary matrix environments retain their existing source alignment. Macro-generated grammar and delarray's delimiter header require explicit syntax or protection.
