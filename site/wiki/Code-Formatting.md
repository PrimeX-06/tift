# Code and docstrip formatting

Algorithm2e and expl3 layout is enabled by default. The corresponding TOML switches are:

```toml
[code]
algorithm2e = true
expl3 = true
```

Both switches default to `true`; set either to `false` to disable its dedicated layout. Dedicated code layout and `.dtx` / `.ins` parsing use tift’s structural scanner and native code printers. Regular `.tex`, `.sty`, and `.cls` files retain tift's formatting stages; the dedicated stage handles only selected algorithm environments and expl3 regions. Dedicated indentation uses spaces with `indent.width`.

## algorithm2e

`code.algorithm2e` selects `algorithm`, `algorithm*`, `algorithm2e`, `algorithm2e*`, `algocf`, and `algocf*`. Known braced control-flow bodies receive structural layout:

```tex
\begin{algorithm}
    \For{$i = 1$}{
        Read $x_i$\;
        Update $s = s + x_i$\;
    }
\end{algorithm}
```

Known calls include `For`, `ForEach`, `ForAll`, `While`, `If`, `ElseIf`, `Else`, `eIf`, and `Repeat`; input/output lines include `KwIn`, `KwOut`, `KwData`, and `KwResult`. Text-mode `\;` separates statements in a confirmed algorithm body. Trailing comments remain attached to their statement. A complete known control-flow call confirms the statement context, allowing ordinary spacing commands in generic algorithm floats to keep their behavior.

The algorithm stage preserves math-mode `\;` and formula source while laying out surrounding statements. The separate `[math]` stage controls formula spacing and line breaks. Current-source redefinitions of core algorithm commands disable dedicated layout for the environment. Dynamically generated command names and custom parameter grammars retain generic TeX handling.

## algorithmic and algpseudocode

The `algorithmic` environment from algorithms/algorithmicx, including algpseudocode, preserves its body as literal pseudocode. Its optional line-number header remains attached. The surrounding algorithm float retains ordinary environment indentation.

## expl3

`code.expl3` selects explicit `\ExplSyntaxOn` / `\ExplSyntaxOff` regions and implicit regions introduced by `\ProvidesExplPackage`, `\ProvidesExplClass`, or `\ProvidesExplFile`. Known definition bodies expand with indentation, and calls follow their argument signatures:

```tex
\ExplSyntaxOn
\cs_new:Npn \sample:n #1
    {
        \tl_set:Nn \l_tmpa_tl {#1}
        \msg_info:nn {sample} {ready}
    }
\ExplSyntaxOff
```

The enabled stage uses expl3's ignored source whitespace. Expl3 protection applies when the switch is disabled. Dynamic catcode regions, literal content, configured raw regions, and formatting directives remain protected.

Fixed character-category assignments such as `\char_set_catcode_other:N \%` enter the same scoped protection as primitive `\catcode` assignments. The affected source retains its exact bytes until the enclosing scope restores the category codes.

## Documented source and installation scripts

Directory scans and single-file formatting recognize `.dtx`, `.fdd`, and `.ins`. `.fdd` uses the documented TeX source workflow. DTX parsing preserves documentation margins at column zero, docstrip guards such as `%<*package>` / `%</package>`, and `macrocode` markers. INS parsing uses package-source syntax and normalizes indentation in existing multiline `\generate` bodies:

```tex
\generate{
    \file{sample.sty}{\from{sample.dtx}{package}}
    \file{extra.sty}{\from{sample.dtx}{extra}}
}
```

Preserve layout retains a single-line `\generate{\file{...}{...}}`. Installation preambles and postambles remain literal generated-file data, including their internal spaces and line breaks.

DTX documentation paragraphs use tift's `wrap` and `sentences` options. For example, `wrap.mode = "rewrap"` joins old documentation lines and wraps to `wrap.columns`. Every output line receives a `% ` margin, whose two columns count toward the width budget. Double-percent comments, guards, `macrocode` markers, protected regions, and atomic math keep their boundaries.

The docstrip code layer uses the native code printer with `indent.width`, `code` switches, content protection, and global file-whitespace settings. Its documentation layer uses `wrap` / `sentences`. The separate native math pass also applies to eligible formulas in `.dtx` and `.ins`; installation preambles and postambles retain their literal data. Native named indentation, structural breaks, argument rules, and `alignment.justification = "columns"` apply in ordinary TeX stages; these rules currently do not enter the docstrip code layer.

Documentation and executable code have separate mathematical views with offsets into the original file. Short verbatim declared by `ltxdoc` keeps characters such as `$` literal inside its delimiters. `\DoNotIndex` arguments retain their role as macro-name data. Code comments remain attached to their statements, including `%^^A` comments that could acquire a different meaning at the documentation margin.

## Literal code protection

Literal protection covers supported listings, minted, FancyVerb, PythonTeX, newverbs, codehigh, scontents, and tcolorbox readers. Recognized factories can declare custom raw environments and inline commands. Package context selects ambiguous names such as `codebox` and `python`; source redefinitions and explicit syntax rules determine which reader applies. Formatting retains the code body's spaces, special characters, and line breaks.

## Math and protection

Complete math regions and protected content are opaque to the dedicated printer. This includes standard math delimiters and environments, `\ensuremath`, fractions, roots, and other known math-argument commands. Their original source is restored after code layout. The separate native math stage is enabled by default for ordinary TeX and eligible docstrip formulas; it controls recognized formula spacing and line breaks. Set `[math] enabled = false` to retain formula-internal spacing and line breaks. Ordinary TeX indentation and outer math alignment continue to apply.

Formatting controls, verbatim, and configured raw content preserve internal indentation, trailing spaces, and line breaks. Ordinary selected code fragments retain their layout if dedicated parsing or opaque restoration cannot be completed safely. BOM, CRLF, and final newlines follow global whitespace settings.

See [configuration](Configuration) for formatting controls and file-specific settings.
