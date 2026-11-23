# Supported syntax and protection

tift recognizes structural TeX/LaTeX syntax and supported literal declarations. Built-in signatures keep complete package headers together and identify body ownership. Explicit syntax settings and custom declarations have priority.

## File types

File extensions are recognized without regard to letter case. Explicit files, directory scans and `--stdin-filename` use the same selection.

| Source | Extensions |
|---|---|
| Documents and drivers | `.tex`, `.ltx`, `.drv` |
| TikZ fragments | `.tikz` |
| Packages, definitions and font declarations | `.sty`, `.def`, `.cfg`, `.fd` |
| Classes and class options | `.cls`, `.clo` |
| BibLaTeX styles, languages and data models | `.bbx`, `.cbx`, `.lbx`, `.dbx` |
| Bibliography data | `.bib` |
| Documented TeX and font definitions | `.dtx`, `.fdd` |
| Docstrip installation scripts | `.ins` |

Definition and style files receive package source handling, including internal `@` names. `.fdd` receives documented source handling with comment margins, docstrip guards and `macrocode` boundaries. File-specific settings remain available through normal path overrides.

## Package families

| Family | Representative coverage |
|---|---|
| Quotations | csquotes `displayquote`, `displaycquote`, foreign-language and hyphenation-aware display variants; citation, attribution and punctuation arguments stay in the environment header while quotation text remains formattable prose |
| Tables and math | `tabular`, `tabular*`, long tables, amsmath, mathtools, tabularray, NiceArray/NiceMatrix, `tikzcd`, and TikZ matrix bodies |
| Drawing | PGF/TikZ paths, literal foreach, pgfplots, pgfgantt, common diagram utilities, PSTricks/PGF environments |
| Code and literal data | verbatim, FancyVerb/fvextra, listings, minted, moreverb, tcolorbox code factories, PythonTeX, newverbs, codehigh, scontents, filecontents, luacode/luadraw, and algorithmic/algpseudocode |
| Code layout | Algorithm2e statements, expl3 definitions, l3keys and kvoptions definition bodies; `.dtx` and `.ins` docstrip source layout |
| Commands and branches | Complete xparse signatures and processors, etoolbox patches and hooks, iftex/ifpdf/ifxetex/ifluatex switches and xifthen tests |
| Layout and floats | minipage, adjustbox, framed/mdframed, wrapfig, multicol, paracol, subcaption/subfig, caption headers, textpos positioned text, fancybox wrappers |
| Typography and annotations | epigraph, lettrine, relsize, makecell, pifont, cancel, diagbox, marginnote, nicefrac/xfrac, ulem, afterpage and manyfoot setup |
| Lists and documents | standard lists, supported theorem wrappers, Beamer/ltx-talk, exam-zh, ElegantBook/ElegantPaper/ElegantNote |
| Bibliography and indexing | natbib `citet/citep/citeauthor/citeyear` and biblatex citation/declaration signatures; makeidx, imakeidx, splitidx, indextools, index, idxlayout, showidx |
| References and terminology | varioref, zref-user, xr-hyper, glossaries, glossaries-extra, acro, acronym, prettyref, fancyref, bibunits |
| Language, theorem and review tools | babel, polyglossia, translations, environ, thmtools, ntheorem, thmbox, todonotes, changes, fixme |
| Scientific notation and proofs | siunitx, chemformula, chemfig, ebproof, bussproofs, mathpartir, interval, derivative, diffcoeff, unicode-math, witharrows, equation helpers, and physics2 matrices |
| Visual and scientific source | OCG layers, animation frames, chemistry reactions, Feynman diagrams, supported raw plotting readers |
| CJK authoring | ctex, xeCJK, xeCJKfntef, xpinyin, zhnumber, zhlipsum, CJK, CJKfntef, CJKnumb, and luatexja-fontspec |

Coverage applies to documented literal forms. Package loading context, local redefinitions, explicit rules, and protection constrain the selected behavior. The [source registries](https://github.com/PrimeX-06/tift/tree/main/src/packages) list recognized package commands and environments.

csquotes display environments keep their metadata separate from the quoted body. For example, `displaycquote` reads optional prenote and postnote slots, a citation key, and optional punctuation before the body; foreign-language variants read the language first. Inline commands such as `\textquote`, `\foreigntextquote` and `\textcquote` use their package-specific argument counts so the following prose is not absorbed into the quotation.

Static column specifications cover 54 table and array environments. Tabularray supports ordered row/column/cell overrides, selectors, and span-aware measurement. See [table column alignment](Table-Alignment) for recognized grammar and fallback behavior. TikZ fragments with a `.tikz` extension use the normal TeX formatting pipeline.

## Typography and positioned text

`textblock` and `textblock*` retain `{width}[reference position](coordinates)` before their body. `makecell` and `thead` support stars and alignment options; `multirowcell` and `multirowthead` retain row count, offset and alignment. `diagbox` accepts two or three braced labels. `marginnote` reads `[left text]{right text}[vertical offset]`, and `sfrac` reads `[instance]{numerator}[separator]{denominator}`.

`cancel`, `nicefrac` and `sfrac` follow their surrounding text or math mode. Mathematical contents receive operator spacing, while prose punctuation retains its authored spelling. `mathlarger`, `mathsmaller` and `cancelto` have mathematical arguments. Nested explicit formulas remain formattable inside cell, epigraph and note text.

`dinglist`, `dingautolist`, `Pilist`, `Piautolist` and `epigraphs` receive list indentation. `afterpage` code, page box definitions, underline settings and footnote hooks retain delayed contents. Manyfoot setup commands have fixed signatures; generated footnote stream names can use explicit `syntax.commands` rules.

CJK font declarations retain leading and trailing font options. Pinyin, Chinese numeral, and CJK font commands keep their settings together while their text arguments remain available for formatting. CJK, CJK*, and pinyinscope environments retain their headers, and xeCJKfntef/CJKfntef annotations preserve nested prose and formulas.

## Package arguments and context

Exercise and date commands from xsim and datetime2 retain setup data and date fields. Import and subfiles paths retain their contents. Scientific commands distinguish equation style settings, rule labels, and matrix templates from mathematical operands.

Reference commands distinguish label and file operands from printed text, even
when optional notes precede the label or options follow a text argument.
Glossary and acronym declarations retain stored settings. Direct descriptions,
link text, review notes and translation branches remain available for ordinary
text and nested formula formatting. `acronym` bodies receive list indentation;
`bibunit` keeps its bibliography style header attached.

Scientific commands separate literal number, unit, chemical formula and font
settings from mathematical arguments. Derivative switches, embellishments and
evaluation delimiters use the shared argument grammar. Proof commands distinguish
text boxes from mathematical statements. WithArrows options and arrow labels
retain their separate roles, including repeated leading options.

Loaded packages and the enclosing environment resolve shared command names.
For example, `siunitx`'s `\qty` takes number and unit operands, and ebproof's
short `\hypo` / `\infer` forms acquire their proof meaning inside `prooftree`.
Conflicting or conditional package loading produces a located warning and
preserves the affected call. Local redefinitions and explicit syntax settings
take priority.

## Literal regions

Verbatim environments, inline `\verb`, `\lstinline`, supported minted commands, and configured raw regions preserve their content. Raw code factories such as `\newminted`, `\lstnewenvironment`, and supported tcolorbox listing declarations can introduce additional literal environments.

The `fancyvrb-ex` environments `Example`, `CenterExample`, `SideBySideExample`, `PCenterExample`, and `PSideBySideExample` preserve their recorded source. Their option and coordinate headers remain attached to the environment. This retains indentation used by `gobble` and the exact code displayed in a compiled document.

Local environment definitions that activate `\obeyspaces`, `\obeylines`, or supported category-code changes protect their affected invocations. Definitions inside expl3 regions participate in the same source-order and scope rules. Redefinitions can restore ordinary formatting; definitions in inactive or literal content do not change later calls.

Moreverb's raw `listing` uses package context; minted's listing float receives normal layout. Embedded `\directlua` and `\latelua` retain source whitespace. Physics delimited matrix arguments remain opaque to outer cell and row scans. Filecontents bodies preserve tabs, trailing spaces, and blank lines.

LilyPond environments and the `\lily` / `\lilypond` recording commands retain their embedded music source. A fixed-signature local wrapper can inherit this protection when source analysis proves that every parameter occurrence stays inside an established recording argument. Parameters passed to delayed TeX hooks retain normal TeX syntax, including formattable mathematics.

Literal `\lynewenvironment` declarations supply the custom music environment's header and recording semantics. Existing environment wrappers converted with `\lyenv` retain their argument data, and `\lysetverbenv` stores its opening and closing code as data. Local scopes, redefinitions, and uncertain conditional definitions determine which recording rule applies at each invocation.

## Formatting controls

```tex
% tift: off
  literal source stays here
% tift: on

  preserve this line % tift: skip

% tift: skip
  preserve the next physical line
```

Spaces after `%` and `tift:` are required. Both off/on marker lines are protected, nested regions are supported, and unmatched off protects the rest of the file. A standalone skip preserves itself and the next line; a trailing skip preserves its own line. Markers in literal code keep their data role.

For custom raw commands and environments, use `protect` and `syntax` configuration. Name arrays replace defaults, so retain built-ins needed by the project. Replacements with `scope = "all"` can deliberately change protected contents.

## Macros and conditionals

Collected environments from `\NewEnviron` / `\RenewEnviron`, theorem factories
such as `\declaretheorem` and `\newboxtheorem`, and supported Acro and derivative
command factories publish their declared argument boundaries. A call uses the
declarations visible at its own source position. Local scopes restore previous
bindings, and local project imports become effective at their loader.

Reference and translation conditionals keep their printable alternatives
visible. Definitions and package loads inside an unevaluated alternative remain
uncertain, including global assignments. An ordinary source redefinition can
replace a built-in conditional meaning.

Literal xparse declarations retain their complete signatures: mandatory and optional arguments, defaults, stars and token switches, embellishments, custom delimiters, verbatim arguments, environment bodies, and processors. The same argument boundaries apply to local declarations, imported project declarations, and explicit `syntax.commands` or `syntax.environment-arguments` signatures. Default values and processor code retain their source tokens.

Natbib citations accept their two optional notes and citation-key argument. Subcaption's `subfigure`, `subtable`, and `subcaptionblock` retain `[position][height][inner position]{width}` before their content. Etoolbox patch and hook commands retain their target/search/replacement/inserted code; immediate success and failure branches remain available for formatting.

L3keys and kvoptions declarations retain delayed definition values and their complete argument boundaries. Engine switches from iftex, ifpdf, ifxetex, and ifluatex have unknown runtime outcomes during source formatting; their alternatives receive structural branch handling. Xifthen `ifthenelse` tests keep the test expression together with two branch arguments.

Safe LaTeX definition bodies receive indentation while text and reflow-sensitive layout remain protected. Primitive and complex definitions receive conservative protection. `protect.format-macro-bodies = false` preserves every body exactly.

Known literal conditionals include `\iftrue`, `\iffalse`, `\ifnum`, `\ifdim`, `\ifodd`, and integer `\ifcase`. Literal numeric operands, dimensions, `\unless`, and supported aliases constrain inactive branches. Exact listings switches such as `\lst@ifdisplaystyle` retain both possible branches unless a local assignment determines their meaning. Unknown outcomes retain the smallest interval needed for possible structures to agree. Active source still receives structural validation.

Literal constructed command names and zero-argument macro values support bounded inference. Parameterized expansion, registers with unknown values, arbitrary execution, and complex dynamic package loading require explicit syntax or protection. Recognized expl3 regions receive dedicated formatting by default; `code.expl3 = false` preserves them. Dynamic catcode regions remain conservatively protected and may produce warnings. See [code and docstrip formatting](Code-Formatting) for options and examples.

## Current limits

- Source analysis covers bounded literal syntax; full TeX execution and arbitrary expansion are outside the implementation.
- Complex macro-generated table grammar, loop headers, or arguments can require explicit signatures and protection.
- Project discovery covers local imports, with bounded cycles and expansion; installed TeX search-path resolution and `\import` need additional configuration.
- Regular expressions use Rust engines with bounded enhanced features. Executable Perl replacements and full Perl regex compatibility are outside the implementation.
- YAML configuration and complete command-line compatibility with other formatters are outside the configuration interface.
- Formatting checks cover source behavior; semantic layout across every package combination requires document compilation and review.

Use `--diff` to inspect changes, `--trace-detail` to identify the formatting stage, and focused name rules or literal regions for custom grammars. [Troubleshooting](FAQ) provides practical diagnosis steps.
