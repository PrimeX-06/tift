# BibTeX and BibLaTeX

The same `[bibtex]` settings apply to BibTeX and BibLaTeX `.bib` files. Default formatting aligns field assignments while preserving source name case, entry order, field order, and value expressions. Entry sorting and data cleanup remain disabled by default.

Generate an editable default configuration, inspect the effective settings for a bibliography, and preview its formatting:

```sh
tift --dump-config > tift.toml
tift --show-config references.bib
tift --diff references.bib
```

```toml
[bibtex]
normalize = true
name-case = "preserve"
sort = false
```

Normalization aligns `=` assignments. Entry types and regular field names keep their authored case, including mixed case within one file. `name-case = "lower"` or `"upper"` applies a uniform spelling to entry types and regular field names, including BibLaTeX types and custom fields. Citation keys, `@string` macro names, macro references, and values retain their case. Field and entry order stay as authored until their ordering settings are enabled. Set `normalize = false` to use uniform assignment spacing; the selected name-case policy remains effective.

| Setting | Default | Behavior |
|---|---|---|
| `normalize` | `true` | Apply assignment normalization and alignment |
| `name-case` | `"preserve"` | Preserve entry-type and regular field-name case; `"lower"` or `"upper"` selects uniform case |
| `align-fields` | `true` | Align regular `=` assignments when normalization is enabled |
| `value-delimiters` | `"preserve"` | Keep values as authored, or request `"braces"` or `"quotes"` for safe literal pieces |
| `field-order` | `[]` | Preserve fields, use `["canonical"]`, use `["alphabetical"]`, or provide preferred field names |
| `sort` | `false` | Sort eligible consecutive regular entries |
| `sort-by` | `["key"]` | Use citation keys, entry types, or field names as successive sort criteria |
| `case-sensitive` | `false` | Compare sort values case-insensitively by default |

## Name case

```toml
[bibtex]
name-case = "upper"
```

```bibtex
@MvReference{MixedKey,
    Title = {Keep This Title},
    author+an = {1=highlight},
    CustomField = JournalMacro # " Supplement",
}
```

formats as:

```bibtex
@MVREFERENCE{MixedKey,
    TITLE       = {Keep This Title},
    AUTHOR+AN   = {1=highlight},
    CUSTOMFIELD = JournalMacro # " Supplement",
}
```

Directive types such as `@string`, `@preamble`, and `@comment` follow the same name-case policy. String definitions retain their macro-name spelling, and comment bodies retain their contents. BibLaTeX annotation fields such as `author+an` and custom fields use the ordinary field layout.

## Value expressions

Values keep their whitespace and expression structure. Bare macro names, numbers, and `#` concatenation remain intact. `@string` and `@preamble` expressions preserve their delimiters. Raw fields `url`, `doi`, `eprint`, `file`, and `pdf` retain their delimiters under every delimiter setting. Optional cleanup can edit selected literal values as described below.

With `value-delimiters = "braces"`, a quoted literal piece becomes a braced literal piece. With `value-delimiters = "quotes"`, a braced piece is converted only when its outer level contains no quotation mark. Nested quotes protected by braces and TeX accent commands retain their spelling. Unsafe pieces stay as authored.

```toml
[bibtex]
normalize = true
align-fields = true
value-delimiters = "braces"
```

```bibtex
@ARTICLE{Example, TITLE="An {Example}", AUTHOR="Doe, Jane", YEAR=2026, month=jan}
```

formats as:

```bibtex
@ARTICLE{Example,
    TITLE  = {An {Example}},
    AUTHOR = {Doe, Jane},
    YEAR   = 2026,
    month  = jan,
}
```

## Field order

Field ordering is separate from entry sorting:

```toml
[bibtex]
field-order = ["title", "author", "date", "year"]
```

Listed fields appear first in that order. Unlisted fields retain their relative source order. `field-order = ["alphabetical"]` sorts all regular field names alphabetically. `field-order = ["canonical"]` uses a shared title-first bibliography order, followed by unknown fields alphabetically. Field-name comparisons ignore case. Repeated fields keep their relative order, and attached field comments travel with their fields. `@string` assignments retain source order.

## Entry sorting

Enable sorting by citation key:

```toml
[bibtex]
sort = true
```

Use author, descending year, title, and then citation key:

```toml
[bibtex]
sort = true
sort-by = ["author", "-year", "title", "key"]
```

Each criterion is ascending by default. Prefix it with `-` for descending or `+` for explicit ascending. Arbitrary regular fields such as `publisher`, `journaltitle`, `sorttitle`, and `keywords` can be used. `entry-key` aliases the citation key; `entry-type` compares entry types. At least one criterion is required.

Author-like fields (`author`, `editor`, `translator`, `sortname`) compare personal family names first, handling comma-form names, juniors, lowercase name particles, and corporate names protected by braces. Successive authors break ties. Comparisons collapse whitespace without changing the stored values. Alphabetic ordering is deterministic Unicode text comparison; it does not apply locale-specific bibliography collation.

`year` uses the `year` field, falling back to the first four digits of a BibLaTeX `date`. Numeric `year`, `volume`, and `number` values compare numerically. Other values compare as text. Missing values follow present values in both ascending and descending order. Equal criteria preserve authored order unless a later criterion resolves the tie.

Earlier `@string` definitions are used when deriving comparison text, including concatenated literal and macro pieces. The expressions in the output retain their original macro references.

String expansion for sorting has an allocation budget proportional to the input size. If an expression exceeds that budget or depends on an unresolved string, tift preserves the expression and reports a warning at its source position. Its entry forms a sorting boundary; other safely resolved runs continue to sort.

## Ordering boundaries

Entries stay on the same side of `@string`, `@preamble`, `@comment`, outside comments, non-entry text, and protected regions. Sorting preserves existing source-relative order between entries linked by `crossref`, `xref`, or `xdata`; independent entries can move around those constraints. Duplicate citation keys remain fixed sorting boundaries, preserving duplicate precedence. Optional cleanup can consolidate compatible duplicate entries before sorting. Blank lines follow the configured whitespace limits, and multiline value content retains its source whitespace.

Use `% tift: off` / `% tift: on` or `% tift: skip` to protect bibliography content. Directive-like text inside quoted or braced values retains its data role.

## Optional data cleanup

Configure data edits under `[bibtex.cleanup]`. All cleanup switches default to `false`, and author lists have no default limit:

```toml
[bibtex.cleanup]
enabled = false
months = false
sentence-case-titles = false
remove-duplicate-fields = false
remove-empty-fields = false
merge-duplicate-entries = false
normalize-urls = false
escape-special-characters = false
generate-missing-keys = false
# max-authors = 8 # Optional positive limit; omitted means unlimited.
```

Set `enabled = true` and select the operations you want. Enabling the group alone keeps every operation disabled. Setting `enabled = false` disables the whole group, including an author limit configured with `max-authors`. Omitted options inherit their defaults.

For example, normalize months and remove empty fields:

```toml
[bibtex.cleanup]
enabled = true
months = true
remove-empty-fields = true
```

| Setting | Behavior |
|---|---|
| `months` | Convert a single quoted or braced English month name, abbreviation, or month number to the standard `jan`–`dec` macro. Bare numeric months also convert. Custom macro references and concatenations remain intact. |
| `sentence-case-titles` | Repair entirely uppercase `title`, `booktitle`, and `journaltitle` values using sentence case. Nested braces protect names and acronyms. Mixed case titles, TeX command names, and mathematical values remain intact. Short uppercase initials, recognized longer acronyms, and digit-bearing terms retain their spelling. Ambiguous longer names still need braces. |
| `remove-duplicate-fields` | Remove identical repeated assignments using case insensitive field-name matching. Preserve conflicting assignments and report their source positions. Comments from removed assignments remain in the entry. |
| `remove-empty-fields` | Remove quoted or braced values containing only whitespace. Preserve attached comments. Macro expressions remain intact. |
| `merge-duplicate-entries` | Consolidate identical entries with the same exact citation key, case insensitive entry type, and complete field set. Preserve comments and keep complementary, subset, and conflicting records separate with a warning. |
| `normalize-urls` | Trim literal `url` and `doi` values, unwrap a complete `\url{…}` wrapper, and remove TeX escapes from `%`, `&`, `#`, and `_` within those values. Values containing unresolved TeX commands remain intact with a warning. |
| `escape-special-characters` | Escape raw `%`, `&`, `#`, `_`, `~`, and `^` in descriptive literal fields, including titles, notes, names, and publisher metadata. Preserve existing escapes, command operands, and mathematical values. URL and DOI values use `normalize-urls`. |
| `generate-missing-keys` | Fill an empty citation key using the first author's surname, year, and first title word. Retain Unicode letters and numbers. Add deterministic numeric suffixes to avoid existing keys. Existing keys and references remain intact. |
| `max-authors` | Retain the first positive number of authors and append the BibTeX marker `and others`. Braced corporate names keep embedded `and` text. Omit this setting to retain complete author lists. |

Quoted and braced literal values support safe data edits. Expressions such as `JournalMacro # " Supplement"` keep their macro references and concatenation structure. A requested cleanup operation that needs a single literal value reports a warning when it cannot safely edit the expression.

### Cleaning a new entry

Enable title repair, missing-key generation, month normalization, empty-field removal, and an author limit:

```toml
[bibtex.cleanup]
enabled = true
months = true
sentence-case-titles = true
remove-empty-fields = true
generate-missing-keys = true
max-authors = 2
```

```bibtex
@article{,
  author = {Smith, Jane and Doe, John and Roe, Alex},
  title = {RESEARCH WITH {DNA}},
  year = 2024,
  month = {January},
  note = {},
}
```

formats as:

```bibtex
@article{Smith2024Research,
    author = {Smith, Jane and Doe, John and others},
    title  = {Research with {DNA}},
    year   = 2024,
    month  = jan,
}
```

Generated keys use the cleaned fields. Use the generated key when adding a citation to the new entry. If a key already exists, tift retains it.

Delimiter conversion remains controlled by `[bibtex].value-delimiters` and can be combined with cleanup:

```toml
[bibtex]
value-delimiters = "braces"

[bibtex.cleanup]
enabled = true
months = true
```

### Duplicate records and conflicts

Entries with the same key can merge within a consecutive run of regular entries. String definitions, preambles, standalone comments, formatting directives, and other source text form boundaries. Different citation keys retain their identities.

With `merge-duplicate-entries = true`, these complementary records remain separate:

```bibtex
@article{study, title={A study}, year=2024}
@article{study, title={A study}, doi={10.1234/example}}
```

Both records retain their own fields and tift reports a warning. Complete identical records can consolidate. Repeated assignments keep their authored order, and signatures compare actual values in a deterministic ordered index.

### Diagnostics and preservation

Cleanup warnings identify source positions for conflicting fields, ambiguous records, and expressions that cannot be safely edited. Preview changes with `tift --diff references.bib` before applying them. Protected bibliography content retains its bytes.

Malformed active input produces an error diagnostic and retains the complete original file. Successful formatting and cleanup are idempotent: formatting the result again produces the same text.
