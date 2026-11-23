# Project declaration context

The CLI prepares command and environment declarations from the selected source and supported local dependencies. This gives custom syntax a consistent header and protection model when formatting a single chapter.

## Selecting the root

```tex
% !TEX root = ../main.tex
```

The path is relative to the file containing the comment. Leading whitespace, case variations in `TEX root`, and quoted paths are accepted. Protected literal content does not select a root. Explicit root chains are followed; a self-reference terminates the chain. Missing roots, empty paths, and cycles between distinct roots produce errors.

An unmarked chapter uses its own input context and supported sibling-list fallback. The CLI does not search the workspace for a parent main file. Editor stdin supports root comments through `--stdin-filename`, including a filename that has not been saved.

## Local imports

The scanner follows supported literal `\input` and `\include` references. Paths without `.tex` are accepted. Candidates are tried relative to the selected root directory, then the importing file's directory. Missing inputs are skipped.

Local class/package imports include `\documentclass`, `\LoadClass`, `\LoadClassWithOptions`, `\usepackage`, `\RequirePackage`, and `\RequirePackageWithOptions`. A local `.cls` has priority over built-in adaptation of that class name. Installed TeX search paths are outside this lookup.

```tex
% main.tex
\documentclass{article}
\input{config/set}
\begin{document}
\input{chapters/one}
\end{document}
```

Put `% !TEX root = ../main.tex` in `chapters/one.tex` to use declarations from `config/set.tex`. Formatting retains the input commands. Dependency reads add declaration context; they do not add formatting targets.

## Literal path macros

Known zero-argument macros can supply paths:

```tex
\def\chapterfile{chapters/one}
\input{\chapterfile}
```

Source order, earlier imports, scoped bindings, `edef`/`xdef` snapshots, and `let` aliases constrain literal values. Parameterized macros, uncertain branches, and execution-dependent expressions require explicit settings.

`syntax.input-ignore-extensions` skips common image, audio, and video suffixes during input/include declaration reads. Companion sources such as `.pdf_tex` remain eligible. A custom array replaces defaults; `[]` allows all suffixes. The option leaves source input commands intact and does not filter local class/package imports.

## Supported declarations

Fixed literal LaTeX definitions and supported xparse `m`, `o`, and `O{default}` slots supply header signatures. List wrappers, supported ordinary tcolorbox factories, raw-code factories, and fixed command definitions contribute context.

Zero-argument literal definitions can alias environment boundaries, including `\def\bea{\begin{equation}}` and `\def\eea{\end{equation}}`. Their calls retain their original spelling and receive the environment's indentation, alignment and mathematical formatting. Declaration order, scopes and imported bindings control the active meaning. Explicit `syntax.environment-aliases` settings take priority. Parameterized, cyclic or ambiguous bindings keep conservative protection and produce located diagnostics; [source analysis](Source-Analysis) describes these limits.

For custom syntax, explicit TOML settings take precedence:

```toml
[syntax.environment-arguments.mytable]
signature = "mom"

[alignment.rules.mytable]
enabled = true

[syntax.commands.raw]
arguments = 1
optional-arguments = false
preserve-body = true
```

Custom tables need named alignment selection. Ordinary tcolorbox factories infer fixed headers with at most one leading optional argument; explicit signatures handle other layouts.

## Ordering and scope

For a chapter referenced by the root, context is collected in input order through its first occurrence, excluding later siblings. A chapter selected by a root but not yet referenced receives the root preamble and its own declarations. Unrelated sibling settings do not supply general declarations.

Conditional, grouped, and non-document-environment inputs are skipped conservatively. This context summarizes declarations for the target; changes at every individual invocation position and arbitrary TeX execution are outside the analysis. Input cycles and literal expansion are bounded. Cross-file comment-factory terminators may need explicit protection.

Configuration discovery and overrides continue to use the target path. `--replacement-only` and BibTeX inputs skip declaration scanning. The library operates on supplied source and configuration; filesystem context is prepared by the CLI.
