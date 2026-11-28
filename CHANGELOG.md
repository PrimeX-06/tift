# Changelog

## 0.2.2

- Large books and theses start formatting sooner. Deeply nested chapter files no longer cause a stack overflow.
- Formatting again preserves custom comment blocks defined in local style files. It also keeps spacing in code listings and fixes unwanted changes to some equations and tables on a second pass.
- Fixed false “unclosed table” errors in documents that use custom commands with optional arguments. When a delimiter error is confined to one part of a file, tift points to that part and formats the rest.
- When you format an unsaved chapter in your editor, tift now respects the input encoding set by the parent XeTeX document.

## 0.2.1

- Read nested braces in xparse verbatim arguments and unbraced text-font arguments. Recognize balanced environment wrappers across file imports and explicit TeX group endings in wrapper definitions.
- Parse literal brackets in prose and simple TeX groups that cross an environment ending. Preserve doc macrocode, sim-os-menus terminal listings, and naive-ebnf grammar bodies.
- Indent nested Forest nodes and multiline conditional branches at their structural depth while preserving existing tree line breaks in the default mode.
- Recognize Forest's macro memoization command, preserve listings in its manual, and handle delayed code written by `\addtocontents`.
- Preserve numberedblock and PGF manual listings, path arguments in URL wrappers, and Xy-pic matrices. Track minipage wrappers, plain TeX loops, and algorithmicx block declarations with an empty starting name.
- Read legacy documents with explicit inputenc or CJK encodings and local inputs preceded by a literal XeTeX encoding assignment. Preserve the document's original encoding when formatting.
- Preserve spacing around sized math delimiters and handle a minus sign after a conditional or divisibility bar when wrapping equations.
- Support atomic in-place writes to long Windows paths and reduce repeated scanning of TeX commands and arguments.

## 0.2.0

- Format lines added or edited since a Git revision with `--changed-since <REV>`. Run `tift --diff --changed-since HEAD` to preview formatting across staged and unstaged edits. New, non-ignored input files are formatted in full.
- When wrapping display math, prefer line breaks between complete equations or inequalities. Preserve constructed control sequences and sized `\hbox` expressions during math layout.
- Keep references such as `Section \ref{sec:intro}`, `Eq. \eqref{eq:main}`, and `page \pageref{app:data}` together when they fit on a line. Preserve arara directive lines when wrapping comments.
- Preserve spaces in inline verbatim code inside BibTeX field values.
- Try different indentation widths or tabs in the browser Playground and see the formatted output update as you edit.

## 0.1.9

- Format classic BibTeX `.bst` styles with indented function bodies, compact field lists, and optional trailing-comment alignment.
- Format supported source types declared in `filecontents` and `filecontents*` using the same settings as standalone files. Set `bibtex.format-filecontents = false` to preserve the embedded source.
- Indent matched algpseudocode branches, loops, functions, and procedures inside `algorithmic`. Set `indent.algpseudocode = false` to preserve the original indentation.
- Keep consecutive same-row booktabs `\cmidrule` commands together by default. Set `breaks.split-cmidrules = true` for one rule per line.
- When wrapping prose, prefer nearby sentence endings and keep reference names and common number–unit pairs together. Enable `wrap.avoid-lone-last-word` to keep a short final pair of English words together. Wrapped comment lists use hanging indents.

## 0.1.8

- Align equation pairs and trailing labels in `align`, `alignat`, `aligned`, and related displays within the configured column budget. Use the three native column directions in `eqnarray`.
- Align tables using Unicode display widths and `\multicolumn` spans. Align equals signs in selected multiline command options by default.
- Finish nonempty, editable files with a newline by default. Set `output.final-newline = "preserve"` to retain the original end of file.
- Configure comment wrapping and alignment with `comments.wrap` and `comments.align-trailing`. Set file endings in `[output]`, the blank-line limit with `whitespace.blank-lines`, and replacement matching with `replacements[].match`.
- Format embedded bibliographies in `filecontents` and `filecontents*` using the same settings as standalone BibTeX files. Set `bibtex.format-filecontents = false` to preserve the embedded source.
- Align literal whitespace-separated PGFPlots tables by default. Set `breaks.pgfplots-tables = false` to preserve inline data or `breaks.pgfplots-named-tables = false` for named tables.
- Add source layout options for bibliography lists and concatenations, repeated command arguments, TikZ paths, and plotting coordinates.

## 0.1.7

- Collapse repeated spaces in ordinary LaTeX prose while preserving existing line breaks. Set `wrap.collapse-spaces = false` to keep repeated spaces. Comments, verbatim content, and literal command arguments retain their spacing.
- Collapse repeated spaces in ordinary BibTeX field values while preserving expression syntax, string definitions, and literal fields such as URLs and DOIs.
- Try LaTeX and BibTeX formatting in the [browser Playground](https://primex-06.github.io/tift/#playground), with taller source and output panels on desktop and mobile.

## 0.1.6

- Expand syntax and formatting coverage for expl3 literal data, hook callbacks, opaque source regions, and deeply nested environments.
- Improve repeated-format performance by reusing activity scans, borrowing expl3 call metadata, reducing opaque-marker allocations, and indexing cross-argument boundaries.
- Add regression coverage and release-mode benchmarks for syntax stability and real-corpus throughput.

## 0.1.5

- Improve TeX parsing for scoped expl3 syntax, conditionals, custom environments, and delayed command arguments. Preserve literal and macro-definition content across indentation, alignment, and math formatting.
- Preserve declared source encodings when reading and writing LaTeX and BibTeX files, including encoding declarations in header comments.
- Serialize concurrent writes and backup rotation for the same output path, using stable lock files alongside atomic file replacements.
- Refine BibTeX cleanup and normalization, and expand support for document, reference, key-definition, and code-listing commands.
- Improve project context, configuration handling, and persistent-cache behavior for the expanded syntax and encoding support.

## 0.1.4

- Configure horizontal formula spacing separately from display line breaks. Keep existing lines, join eligible display bodies onto one line, or wrap to a separate math width with optional operator priorities.
- Enable optional layout for multiline key/value assignments, table column specifications, literal `\foreach` lists, and `forest` and `qtree` bracket trees. These options preserve the previous layout by default.
- Preserve literal bodies in more code and pseudocode environments, including `algorithmic`, and recognize more custom listing declarations from minted, PythonTeX, codehigh, and tcolorbox.
- Expand support for document and scientific commands, including date and exercise settings, CJK authoring, equation helpers, mathpartir proofs, and physics2 matrices. Literal options and metadata retain their contents.
- Keep the hanging indentation of list prose when a long item title wraps. Reduce repeated parsing during indentation and math formatting.

## 0.1.3

- Expanded support for LaTeX packages, CJK writing, custom environments, and locally defined command signatures.
- Added `tift --dump-config` to export a complete starting configuration.
- Added optional cleanup for BibTeX entries, fields, titles, and URLs.

[Earlier releases](https://github.com/PrimeX-06/tift/releases)
