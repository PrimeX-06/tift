# Formula formatting

Formula formatting is enabled by default. Disable the complete math stage with:

```toml
[math]
enabled = false
```

Ordinary indentation and outer equation alignment follow their own configuration.

## Spacing rules

`math.spacing` defaults to `true`. Set it to `false` to retain horizontal formula whitespace while keeping display line-break settings active:

```toml
[math]
spacing = false
```

Known infix binary operators and relations receive source spaces: `a+b` becomes `a + b`, and `a\leqslant b` becomes `a \leqslant b`. Unary signs stay with their operand. Slash gaps are preserved when both sides have a space: `a / b` stays unchanged. One-sided gaps in `a/ b` and `a /b` are removed. Compound relations such as `<=` and `:=` remain intact.

Script attachment gaps are removed, so `x ^ 2` becomes `x^2` and `a _ i` becomes `a_i`. Character operators inside scripts use compact spacing. Control-word operator gaps inside scripts keep their authored form:

| Input | Output |
|---|---|
| `x^{a + b}` | `x^{a+b}` |
| `x_{i = 1}` | `x_{i=1}` |
| `x_{a\in A}` | unchanged |
| `x_{a\to1}` | unchanged |
| `x_{a \to 1}` | unchanged |
| `x_{ a \to 1 }` | `x_{a \to 1}` |

Ordinary paired parentheses retain interior gaps when both sides have a space: `( a )` stays unchanged. One-sided gaps in `( a)` and `(a )` are removed. The same rule applies inside `\left` and `\right` pairs. The sizing commands attach to their delimiter, so `\left (a\right )` becomes `\left(a\right)`. Required control-word separators remain intact, including the space in `\langle a\rangle`. Repeated spaces on the same physical math line collapse to one space, so `a  b` becomes `a b`. Explicit spacing commands and text arguments preserve their contents. Known math arguments such as fractions, roots, and `\ensuremath` use math spacing.

Common unicode-math and AMS symbols receive their corresponding formula spacing. Complete control sequences retain their spelling, including `\inA`.

## Package commands and environments

Mathematical arguments are recognized independently of style, dimension, and declaration arguments. Text arguments inside formulas retain their text spacing.

| Package | Recognized forms | Source behavior |
|---|---|---|
| mathtools | `mathclap`, `mathllap`, `mathrlap`, `cramped` variants, `underbracket`, `overbracket`, `splitfrac`, `splitdfrac`, `prescript` | Mathematical arguments use formula spacing; optional styles and dimensions retain their contents |
| AMS and mathtools | Extensible arrows such as `xleftarrow`, `xrightarrow`, `xleftrightarrow`, and `xmapsto` | Both arrow labels use math spacing |
| mathtools | `dcases`, `rcases`, `drcases`, starred forms, and `cases*` | Align cells; the second column in starred cases retains text spacing |
| breqn | `dmath`, `dmath*`, `darray`, `dgroup`, `dseries`, and starred wrappers | Recognize optional headers; mathematical bodies and explicit nested formulas use their corresponding math modes |
| commath | `od`, `tod`, `dod`, `pd`, `tpd`, `dpd`, `md`, `tmd`, `dmd` | Recognize derivative order, function, and variable arguments |
| tensor | `tensor`, `indices`, `nuclide` | Preserve stars and optional left indices; format mathematical object and index arguments |
| braket | `bra`, `ket`, `braket`, `Bra`, `Ket`, `Braket`, `set`, `Set` | Format mathematical arguments and preserve vertical-bar separators |
| accents | `accentset`, `underaccent`, `undertilde` | Format mathematical arguments |
| relsize | `mathlarger`, `mathsmaller` | Format mathematical arguments |
| cancel | `cancel`, `bcancel`, `xcancel`, `cancelto` | Cancellation contents follow their surrounding mode; `cancelto` has mathematical arguments |
| nicefrac / xfrac | `nicefrac[font]{numerator}{denominator}`, `sfrac[instance]{numerator}[separator]{denominator}` | Follow surrounding text or math mode; settings retain their domains |

```tex
\od[2]{x + y}{t}
\tensor[^a_b]{T + S}{^c_d}
$a \xleftrightarrow[x + y]{n = m} b$
```

Paired-delimiter declarations (`DeclarePairedDelimiter`, `DeclarePairedDelimiterX`, and `DeclarePairedDelimiterXPP`) retain their declaration arguments. Generated custom command names use custom syntax settings. Source redefinitions and explicit syntax rules take precedence. Breqn bodies retain their authored line boundaries.

## Display line breaks

Use `math.wrap` to choose how eligible display bodies handle line breaks:

| Value | Behavior |
|---|---|
| `"preserve"` | Keep authored line boundaries |
| `"single-line"` | Join eligible display bodies onto one line |
| `"break"` | Wrap at safe top-level operators; the default |

```toml
[math]
wrap = "break"
columns = 88 # Omit to inherit wrap.columns.
break-priority = ["relation", "additive"]
```

`break-priority` orders operator classes from highest to lowest preference, with the latest fitting position chosen within each class. Supported classes are `relation`, `additive`, `multiplicative`, and `binary`. Unlisted classes follow the listed classes. The default empty array uses greedy wrapping. Repeated classes are rejected.

With `math.wrap = "break"`, long single-formula displays use `math.columns`, falling back to `wrap.columns`, independently of prose wrapping. Eligible top-level binary operators and relations provide breaks before the operator. Inline formulas, unary operators, parenthesized subexpressions, scripts, fractions, and other structural atoms stay together. `\cdot` stays with its multiplicative term under the default policy; an explicit `multiplicative` preference enables a break before it. `\mid` protects its following condition relations.

Display body indentation follows `indent.rules.displayMath`, `indent.rules.'$$'`, or the corresponding environment rule such as `indent.rules.equation`. Comments, explicit row breaks, protected source, and incomplete sized-delimiter pairs retain their necessary boundaries. Atomic expressions can exceed the configured width.

Math also applies to eligible style, class, DTX, and INS source. DTX documentation margins remain intact; margin-bearing multiline formulas retain their authored line breaks. INS preambles and postambles remain literal generated-file data.

See [code and docstrip formatting](Code-Formatting) for the surrounding code layer, and [configuration](Configuration) for defaults and indentation options.
