//! Literal environment grammar from installed LaTeX packages.
//!
//! Only signatures and structural distinctions that affect formatting live
//! here. User-defined names and expansion-dependent grammar require source
//! declarations or configuration.

use crate::trivia::skip as trivia_end;
use std::collections::HashMap;
use std::ops::Range;

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &[
    // tabularray.sty amsmath library: O{} +b wrappers around +array.
    "+matrix",
    "+bmatrix",
    "+Bmatrix",
    "+pmatrix",
    "+vmatrix",
    "+Vmatrix",
    "+cases",
    "subarray",
    "crampedsubarray",
    "multlined",
    "lgathered",
    "rgathered",
    "xalignat",
    "xalignat*",
    "xxalignat",
    "psmallmatrix",
    "bsmallmatrix",
    "Bsmallmatrix",
    "vsmallmatrix",
    "Vsmallmatrix",
    "psmallmatrix*",
    "bsmallmatrix*",
    "Bsmallmatrix*",
    "vsmallmatrix*",
    "Vsmallmatrix*",
    "RTLcases",
    "NiceArray",
    "pNiceArray",
    "bNiceArray",
    "BNiceArray",
    "vNiceArray",
    "VNiceArray",
    "NiceArrayWithDelims",
    "NiceMatrix",
    "pNiceMatrix",
    "bNiceMatrix",
    "BNiceMatrix",
    "vNiceMatrix",
    "VNiceMatrix",
    "pNiceArrayC",
    "bNiceArrayC",
    "BNiceArrayC",
    "vNiceArrayC",
    "VNiceArrayC",
    "NiceArrayCwithDelims",
    "pNiceArrayRC",
    "bNiceArrayRC",
    "BNiceArrayRC",
    "vNiceArrayRC",
    "VNiceArrayRC",
    // tikzlibrarycd.code.tex:144--150 opens a matrix of math/text nodes.
    "tikzcd",
];

pub(crate) const ADDITIONAL_VERBATIM_ENVIRONMENTS: &[&str] = &[
    // moreverb.sty:22, 83, 96, 140, 155. Its listing name also belongs
    // to minted's ordinary float, so listing is selected from source context.
    "verbatimwrite",
    "verbatimtab",
    "boxedverbatim",
    "listingcont",
    "listingcont*",
    // tcolorbox.sty:2865, 2951; tcblistingscore.code.tex:275, 293;
    // tcbexternal.code.tex:148; tcbdocumentation.code.tex:272--298.
    "tcbverbatimwrite",
    "tcbwritetemp",
    "tcblisting",
    "tcboutputlisting",
    "tcbexternal",
    "dispExample",
    "dispExample*",
    "dispListing",
    "dispListing*",
];

pub(crate) const ADDITIONAL_LIST_ENVIRONMENTS: &[&str] = &[
    "compactenum",
    "compactitem",
    "compactdesc",
    "inparaenum",
    "inparaitem",
    "inparadesc",
    "inparablank",
    "asparaenum",
    "asparaitem",
    "asparadesc",
    "asparablank",
    "tasks",
];

pub(crate) fn is_package_item(name: &str, environment: &str) -> bool {
    name.trim_end_matches('*') == "task" && environment == "tasks"
        || name == "qitem" && environment == "epigraphs"
        || crate::document_environments::is_item(name, environment)
}

fn additional_header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // adjustbox/collectbox: keys precede the collected body. Min/max size
        // wrappers take a width and a height, with a starred total-height form.
        "adjustbox" => "m",
        "minsizebox" | "minsizebox*" | "maxsizebox" | "maxsizebox*" => "mm",
        // mdframed.sty declares one optional key/value argument.
        "mdframed" => "o",
        // framed.sty declares these paragraph wrappers without header arguments.
        "framed" | "shaded" | "shaded*" | "leftbar" | "snugshade" | "snugshade*" | "oframed" => "",
        "titled-frame" => "m",
        // changepage.sty: two mandatory left/right margin adjustments.
        "adjustwidth" | "adjustwidth*" => "mm",
        // wrapfig.sty: [line count]{placement}[overhang]{width}.
        "wrapfigure" | "wraptable" => "omom",
        // multicol.sty: {column count}[heading][space before the columns].
        "multicols" | "multicols*" => "moo",
        // paralist.sty separates optional enum/item labels from descriptions.
        "compactenum" | "compactitem" | "inparaenum" | "inparaitem" | "asparaenum"
        | "asparaitem" => "o",
        "compactdesc" | "inparadesc" | "inparablank" | "asparadesc" | "asparablank" => "",
        // tasks.sty: O{}D(){1}+b. Parenthesized columns belong to the header.
        "tasks" => "op",
        // csquotes.sty: quotation citation and punctuation arguments. Language
        // wrappers prepend the language; cquote forms also consume a cite key.
        "displayquote" => "oo",
        "foreigndisplayquote" | "hyphendisplayquote" => "moo",
        "displaycquote" => "oomo",
        "foreigndisplaycquote" | "hyphendisplaycquote" => "moomo",
        _ => return None,
    })
}

pub(crate) fn has_additional_signature(name: &str) -> bool {
    additional_header_signature(name).is_some()
}

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    if let Some(signature) = additional_header_signature(name) {
        return Some(signature);
    }
    Some(match name {
        // tabularray.sty:8099--8148, all seven wrappers read inner options.
        "+matrix" | "+bmatrix" | "+Bmatrix" | "+pmatrix" | "+vmatrix" | "+Vmatrix" | "+cases" => {
            "o"
        }
        "refsection" => "o",
        "refcontext" => "og",
        "refsegment" => "",
        // amsmath.sty:1004, 1756--1769; mathtools.sty:795, 1651, 1786.
        "subarray" | "crampedsubarray" | "xalignat" | "xalignat*" | "xxalignat" | "spreadlines" => {
            "m"
        }
        "multlined" => "oo",
        "lgathered" | "rgathered" => "o",
        // subcaption.sty:73--110 delegates these boxes to minipage.
        "subfigure" | "subtable" | "subcaptionblock" => "ooom",
        // subcaption.sty:60--70 switches the caption subtype inside a group.
        "subcaptiongroup" | "subcaptiongroup*" => "",
        // mathtools.sty:1084--1100 generates both fenced variants.
        "psmallmatrix*" | "bsmallmatrix*" | "Bsmallmatrix*" | "vsmallmatrix*" | "Vsmallmatrix*" => {
            "o"
        }
        "psmallmatrix" | "bsmallmatrix" | "Bsmallmatrix" | "vsmallmatrix" | "Vsmallmatrix"
        | "subequations" | "algomathdisplay" | "RTLcases" => "",
        // nicematrix.sty:2153, 4085--4149. The final option is adjacent.
        "NiceArrayWithDelims" => "mmomO",
        "NiceArray" | "pNiceArray" | "bNiceArray" | "BNiceArray" | "vNiceArray" | "VNiceArray" => {
            "omO"
        }
        "NiceMatrix" | "pNiceMatrix" | "bNiceMatrix" | "BNiceMatrix" | "vNiceMatrix"
        | "VNiceMatrix" | "NiceMatrixBlock" => "O",
        // Retain names present in earlier nicematrix releases.
        "pNiceArrayC"
        | "bNiceArrayC"
        | "BNiceArrayC"
        | "vNiceArrayC"
        | "VNiceArrayC"
        | "NiceArrayCwithDelims"
        | "pNiceArrayRC"
        | "bNiceArrayRC"
        | "BNiceArrayRC"
        | "vNiceArrayRC"
        | "VNiceArrayRC" => "omo",
        // tcbraster.code.tex:131, 240, 246, 250.
        "tcolorbox" | "tcbraster" | "tcbitemize" => "o",
        // varwidth.sty reads the maximum width before its text body.
        "varwidth" => "m",
        // lineno.sty wrappers control numbering around their paragraph/math body.
        "linenumbers" | "linenumbers*" => "o",
        "internallinenumbers" | "internallinenumbers*" | "linenomath" | "linenomath*" => "",
        // rotating.sty float wrappers accept ordinary placement metadata.
        "sidewaysfigure" | "sidewaystable" => "o",
        "sideways" => "",
        "turn" | "rotate" => "m",
        // verse.sty stanza wrappers and its width-limited verse environment.
        "verse" => "o",
        "altverse" | "patverse" | "patverse*" => "",
        // lscape/pdflscape rotate page content without header parameters.
        "landscape" => "",
        // boxedminipage.sty and quoting.sty collect width/local settings.
        "boxedminipage" => "m",
        "quoting" => "o",
        // parnotes.sty prints its notes after each paragraph.
        "autopn" => "",
        // quotchap.sty saves a chapter epigraph at a caller-selected width.
        "savequote" => "o",
        // sidecap.sty orders relative caption width before float placement.
        "SCfigure" | "SCtable" | "SCfigure*" | "SCtable*" => "oo",
        // parallel.sty uses an optional mode followed by left/right widths.
        "Parallel" => "omm",
        // parcolumns.sty selects a column count after its key list.
        "parcolumns" => "om",
        "tcboxedraster" | "tcboxeditemize" => "om",
        // algorithm2e.sty:2487, 2740--2815; algorithmicx.sty:74.
        "algorithm" | "algorithm*" | "algorithm2e" | "algorithm2e*" | "algocf" | "algocf*"
        | "procedure" | "procedure*" | "function" | "function*" | "algorithmic" => "o",
        // forest.sty:8506 has D(){}; its bracketed tree is body syntax.
        "forest" => "p",
        // tikzlibrarycd.code.tex:125--128.
        "tikzcd" => "o",
        // Ordinary bxjscls and bidi text wrappers have no header arguments.
        "titlepage" | "chapterabstract" | "LTR" | "LTR*" | "RTL" | "RTL*" | "LTRitems"
        | "RTLitems" | "LTRbibitems" | "RTLbibitems" => "",
        // bookcover.cls creates a sequence of bookcovercomponent entries.
        "bookcover" => "",
        "bookcoverelement" => "mmom",
        // lipsum.sty also exposes its paragraph generator as an environment.
        "lipsum" => "o",
        // These signatures also describe the protected opening token.
        "minted" | "tcbexternal" => "om",
        // The ordinary minted float uses only positioning. moreverb's
        // conflicting listing environment is protected from source context.
        "listing" | "listing*" => "o",
        "lstlisting" => "om",
        "verbatimwrite" | "tcbverbatimwrite" | "tcblisting" | "dispExample*" | "dispListing*" => {
            "m"
        }
        "verbatimtab" => "o",
        "boxedverbatim" | "listingcont" | "listingcont*" | "comment" | "tcbwritetemp"
        | "tcboutputlisting" | "dispExample" | "dispListing" => "",
        _ => return None,
    })
}

/// Column templates may contain definitions and conditional tokens.
pub(crate) fn column_spec_slot(name: &str) -> Option<usize> {
    Some(match name {
        "+matrix" | "+bmatrix" | "+Bmatrix" | "+pmatrix" | "+vmatrix" | "+Vmatrix" | "+cases" => 0,
        "subarray" | "crampedsubarray" => 0,
        "NiceArrayWithDelims" => 3,
        "NiceArray"
        | "pNiceArray"
        | "bNiceArray"
        | "BNiceArray"
        | "vNiceArray"
        | "VNiceArray"
        | "pNiceArrayC"
        | "bNiceArrayC"
        | "BNiceArrayC"
        | "vNiceArrayC"
        | "VNiceArrayC"
        | "NiceArrayCwithDelims"
        | "pNiceArrayRC"
        | "bNiceArrayRC"
        | "BNiceArrayRC"
        | "vNiceArrayRC"
        | "VNiceArrayRC" => 1,
        _ => return None,
    })
}

/// Math fragments require their surrounding source tokens to stay adjacent.
pub(crate) fn is_math(name: &str) -> bool {
    matches!(
        name,
        "+array"
            | "+matrix"
            | "+bmatrix"
            | "+Bmatrix"
            | "+pmatrix"
            | "+vmatrix"
            | "+Vmatrix"
            | "+cases"
            | "subarray"
            | "crampedsubarray"
            | "multlined"
            | "lgathered"
            | "rgathered"
            | "psmallmatrix"
            | "bsmallmatrix"
            | "Bsmallmatrix"
            | "vsmallmatrix"
            | "Vsmallmatrix"
            | "psmallmatrix*"
            | "bsmallmatrix*"
            | "Bsmallmatrix*"
            | "vsmallmatrix*"
            | "Vsmallmatrix*"
            | "RTLcases"
            | "NiceArray"
            | "pNiceArray"
            | "bNiceArray"
            | "BNiceArray"
            | "vNiceArray"
            | "VNiceArray"
            | "NiceArrayWithDelims"
            | "NiceMatrix"
            | "pNiceMatrix"
            | "bNiceMatrix"
            | "BNiceMatrix"
            | "vNiceMatrix"
            | "VNiceMatrix"
            | "pNiceArrayC"
            | "bNiceArrayC"
            | "BNiceArrayC"
            | "vNiceArrayC"
            | "VNiceArrayC"
            | "NiceArrayCwithDelims"
            | "pNiceArrayRC"
            | "bNiceArrayRC"
            | "BNiceArrayRC"
            | "vNiceArrayRC"
            | "VNiceArrayRC"
    )
}

pub(crate) fn is_block(name: &str) -> bool {
    crate::packages::visual_languages::is_block(name)
        || crate::packages::links_color_columns::is_block(name)
        || crate::packages::page_styles::is_block(name)
        || crate::packages::typography::is_block(name)
        || (has_additional_signature(name) && !is_box(name))
        || crate::class_environments::header_signature(name).is_some()
        || crate::document_environments::header_signature(name).is_some()
        || crate::additional_package_syntax::is_block(name)
        || matches!(
            name,
            "xalignat"
                | "xalignat*"
                | "xxalignat"
                | "subequations"
                | "spreadlines"
                | "algomathdisplay"
                | "tcolorbox"
                | "tcbraster"
                | "tcbitemize"
                | "tcboxedraster"
                | "tcboxeditemize"
                | "algorithm"
                | "algorithm*"
                | "algorithm2e"
                | "algorithm2e*"
                | "algocf"
                | "algocf*"
                | "procedure"
                | "procedure*"
                | "function"
                | "function*"
                | "algorithmic"
                | "titlepage"
                | "chapterabstract"
                | "LTR"
                | "LTR*"
                | "RTL"
                | "RTL*"
                | "LTRitems"
                | "RTLitems"
                | "LTRbibitems"
                | "RTLbibitems"
                | "listing"
                | "listing*"
        )
}

pub(crate) fn is_box(name: &str) -> bool {
    crate::packages::visual_languages::is_box(name)
        || crate::packages::circuits_timing::is_box(name)
        || crate::packages::tikz_diagrams::is_box(name)
        || crate::additional_package_syntax::is_box(name)
        || matches!(
            name,
            "forest"
                | "tikzcd"
                | "subfigure"
                | "subtable"
                | "subcaptionblock"
                | "adjustbox"
                | "minsizebox"
                | "minsizebox*"
                | "maxsizebox"
                | "maxsizebox*"
                | "varwidth"
                | "sidewaysfigure"
                | "sidewaystable"
                | "sideways"
                | "turn"
                | "rotate"
                | "boxedminipage"
        )
}

/// These wrappers may expose text whose whitespace survives their begin/end.
pub(crate) fn preserves_tight_whitespace(name: &str) -> bool {
    crate::packages::visual_languages::preserves_tight_whitespace(name)
        || crate::packages::circuits_timing::preserves_tight_whitespace(name)
        || crate::packages::tikz_diagrams::preserves_tight_whitespace(name)
        || crate::packages::page_styles::preserves_tight_whitespace(name)
        || crate::class_environments::preserves_tight_whitespace(name)
        || matches!(name, "frame" | "block" | "alertblock" | "exampleblock")
        || matches!(
            name,
            "spreadlines"
                | "NiceMatrixBlock"
                | "forest"
                | "subcaptiongroup"
                | "subcaptiongroup*"
                | "titlepage"
                | "chapterabstract"
                | "LTR"
                | "LTR*"
                | "RTL"
                | "RTL*"
                | "LTRitems"
                | "RTLitems"
                | "LTRbibitems"
                | "RTLbibitems"
                | "adjustbox"
                | "minsizebox"
                | "minsizebox*"
                | "maxsizebox"
                | "maxsizebox*"
                | "inparaenum"
                | "inparaitem"
                | "inparadesc"
                | "inparablank"
        )
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerbatimDeclaration {
    pub names: Vec<String>,
    pub end: usize,
    pub terminator: VerbatimTerminator,
}

pub(crate) use crate::config::VerbatimTerminator;

fn argument(source: &str, cursor: &mut usize, open: u8, close: u8) -> Option<Range<usize>> {
    let start = trivia_end(source, *cursor);
    let end = crate::syntax::balanced_end(source, start, open, close)?;
    *cursor = end;
    Some(start + 1..end - 1)
}

fn option(source: &str, cursor: &mut usize) -> Option<Range<usize>> {
    argument(source, cursor, b'[', b']')
}

fn mandatory(source: &str, cursor: &mut usize) -> Option<Range<usize>> {
    argument(source, cursor, b'{', b'}')
}

fn literal_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && !value
            .chars()
            .any(|c| c.is_whitespace() || "\\{}[]%#,".contains(c)))
    .then(|| value.to_owned())
}

#[derive(Debug)]
pub(crate) struct BoxDeclaration {
    pub name: String,
    pub arguments: crate::config::EnvironmentArguments,
    pub end: usize,
}

fn fixed_box_specification(
    source: &str,
    range: Range<usize>,
) -> Option<crate::config::EnvironmentArguments> {
    let mut result = crate::config::EnvironmentArguments::default();
    let mut cursor = range.start;
    while cursor < range.end {
        cursor = trivia_end(source, cursor);
        if cursor >= range.end {
            break;
        }
        if source.as_bytes()[cursor] == b'+' {
            cursor = trivia_end(source, cursor + 1);
            if cursor >= range.end {
                return None;
            }
        }
        match source.as_bytes()[cursor] {
            b'm' => {
                result.arguments += 1;
                if result.arguments + usize::from(result.optional_arguments) > 9 {
                    return None;
                }
                cursor += 1;
            }
            b'o' | b'O' if !result.optional_arguments && result.arguments == 0 => {
                result.optional_arguments = true;
                let has_default = source.as_bytes()[cursor] == b'O';
                cursor += 1;
                if has_default {
                    mandatory(source, &mut cursor)?;
                    if cursor > range.end {
                        return None;
                    }
                }
            }
            _ => return None,
        }
    }
    Some(result)
}

/// tcolorbox.sty:2571--2624 forwards these forms to LaTeX/xparse environment
/// declarations. Only fixed arity with one leading bracket option is inferred.
pub(crate) fn literal_box_declaration(
    source: &str,
    command: &str,
    end: usize,
) -> Option<BoxDeclaration> {
    if !matches!(
        command,
        "newtcolorbox"
            | "renewtcolorbox"
            | "NewTColorBox"
            | "RenewTColorBox"
            | "DeclareTColorBox"
            | "ProvideTColorBox"
    ) {
        return None;
    }
    let mut cursor = end;
    option(source, &mut cursor);
    let name = literal_name(&source[mandatory(source, &mut cursor)?])?;
    let arguments = match command {
        "newtcolorbox" | "renewtcolorbox" => {
            let count = option(source, &mut cursor);
            let has_count = count.is_some();
            let total = if let Some(count) = count {
                let count = source[count].trim();
                if count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                count.parse::<usize>().ok()?
            } else {
                0
            };
            if total > 9 {
                return None;
            }
            let optional_arguments = has_count && option(source, &mut cursor).is_some();
            crate::config::EnvironmentArguments {
                arguments: total.checked_sub(usize::from(optional_arguments))?,
                optional_arguments,
                ..Default::default()
            }
        }
        "NewTColorBox" | "RenewTColorBox" | "DeclareTColorBox" | "ProvideTColorBox" => {
            let specification = mandatory(source, &mut cursor)?;
            fixed_box_specification(source, specification)?
        }
        _ => return None,
    };
    mandatory(source, &mut cursor)?;
    Some(BoxDeclaration {
        name,
        arguments,
        end: cursor,
    })
}

fn literal_box_factory_chain_end(source: &str, mut cursor: usize, limit: usize) -> Option<usize> {
    while cursor < limit {
        let (command, end) = crate::syntax::command_at(source, cursor)?;
        cursor = literal_box_declaration(source, command, end)?.end;
        if cursor > limit {
            return None;
        }
    }
    Some(cursor)
}

fn box_factory_shape(source: &str, command: &str, end: usize) -> Option<(String, usize)> {
    let document_signature = match command {
        "newtcolorbox" | "renewtcolorbox" => false,
        "NewTColorBox" | "RenewTColorBox" | "DeclareTColorBox" | "ProvideTColorBox" => true,
        _ => return None,
    };
    let mut cursor = end;
    option(source, &mut cursor);
    let name = literal_name(&source[mandatory(source, &mut cursor)?])?;
    if document_signature {
        mandatory(source, &mut cursor)?;
    } else {
        option(source, &mut cursor);
        option(source, &mut cursor);
    }
    mandatory(source, &mut cursor)?;
    Some((name, cursor))
}

fn macro_declaration(command: &str) -> bool {
    matches!(
        command,
        "newcommand"
            | "renewcommand"
            | "providecommand"
            | "DeclareRobustCommand"
            | "DeclareMathOperator"
            | "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "DeclareDocumentCommand"
            | "ProvideDocumentCommand"
            | "newenvironment"
            | "renewenvironment"
            | "NewDocumentEnvironment"
            | "RenewDocumentEnvironment"
            | "DeclareDocumentEnvironment"
            | "ProvideDocumentEnvironment"
    )
}

fn definition_target(source: &str, command: &str, end: usize) -> Option<(String, usize)> {
    let mut cursor = trivia_end(source, end);
    if command.contains("environment") || command.contains("Environment") {
        return Some((
            literal_name(&source[mandatory(source, &mut cursor)?])?,
            cursor,
        ));
    }
    let grouped = source.as_bytes().get(cursor) == Some(&b'{');
    if grouped {
        cursor = trivia_end(source, cursor + 1);
    }
    let (name, end) = crate::syntax::command_at(source, cursor)?;
    cursor = end;
    if grouped {
        cursor = trivia_end(source, cursor);
        if source.as_bytes().get(cursor) != Some(&b'}') {
            return None;
        }
        cursor += 1;
    }
    Some((name.trim_end_matches('*').to_owned(), cursor))
}

fn definition_end(source: &str, command: &str, mut cursor: usize) -> Option<usize> {
    if matches!(command, "def" | "gdef" | "edef" | "xdef") {
        while cursor < source.len() && source.as_bytes()[cursor] != b'{' {
            if source.as_bytes()[cursor] == b'%' {
                cursor = trivia_end(source, cursor);
            } else {
                cursor += source[cursor..].chars().next()?.len_utf8();
            }
        }
    } else if matches!(command, "let" | "futurelet") {
        cursor = trivia_end(source, cursor);
        if command == "let" && source.as_bytes().get(cursor) == Some(&b'=') {
            cursor = trivia_end(source, cursor + 1);
        }
        for _ in 0..if command == "futurelet" { 2 } else { 1 } {
            cursor = trivia_end(source, cursor);
            cursor = crate::syntax::command_at(source, cursor).map_or(
                cursor + source[cursor..].chars().next()?.len_utf8(),
                |(_, end)| end,
            );
        }
        return Some(cursor);
    } else {
        while option(source, &mut cursor).is_some() {}
        if command.contains("Document") {
            mandatory(source, &mut cursor)?;
        }
    }
    mandatory(source, &mut cursor)?;
    if command.contains("environment") || command.contains("Environment") {
        mandatory(source, &mut cursor)?;
    }
    Some(cursor)
}

/// Resolve all ordinary box headers in one source scan. The caller supplies
/// the protection ranges for this exact text and configuration.
pub(crate) fn source_box_argument_map(
    source: &str,
    config: &crate::Config,
    protected: &[Range<usize>],
) -> HashMap<String, crate::config::EnvironmentArguments> {
    // Every supported factory contains one of these two case-sensitive names.
    if !source.contains("tcolorbox") && !source.contains("TColorBox") {
        return HashMap::new();
    }
    // Preserve the old raw-source candidate filter independently of the
    // protected scan. A custom protected range can cut an escaped command
    // boundary, while another raw candidate for its name can still qualify it.
    let mut candidates = std::collections::HashSet::new();
    for (start, _) in source.match_indices('\\') {
        let Some((command, end)) = crate::syntax::command_at(source, start) else {
            continue;
        };
        if matches!(
            command,
            "newtcolorbox"
                | "renewtcolorbox"
                | "NewTColorBox"
                | "RenewTColorBox"
                | "DeclareTColorBox"
                | "ProvideTColorBox"
        ) && !crate::syntax::is_escaped(source, start)
            && let Some((name, _)) = box_factory_shape(source, command, end)
        {
            candidates.insert(name);
        }
    }
    if candidates.is_empty() {
        return HashMap::new();
    }
    let mut cursor = 0;
    let mut depth = 0usize;
    let mut redefined_commands = std::collections::HashSet::new();
    let mut declared_environments = std::collections::HashSet::new();
    let mut selected = HashMap::new();
    let mut readable_factory_end = None;
    while cursor < source.len() {
        let index = protected.partition_point(|range| range.end <= cursor);
        let range = protected.get(index).filter(|range| range.start <= cursor);
        if let Some(range) = range.filter(|range| range.start == cursor)
            && literal_box_factory_chain_end(source, cursor, range.end) == Some(range.end)
        {
            readable_factory_end = Some(range.end);
        }
        if let Some(range) =
            range.filter(|range| range.start != cursor && readable_factory_end != Some(range.end))
        {
            cursor = range.end;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = source[cursor..]
                .find('\n')
                .map_or(source.len(), |n| cursor + n + 1);
            continue;
        }
        if let Some((command, end)) = crate::syntax::command_at(source, cursor) {
            let base = command.trim_end_matches('*');
            let factory = if depth == 0
                && !command.ends_with('*')
                && !config.syntax.commands.contains_key(base)
                && !redefined_commands.contains(base)
            {
                literal_box_declaration(source, base, end)
                    .filter(|_| range.is_none_or(|range| readable_factory_end == Some(range.end)))
            } else {
                None
            };
            if let Some(declaration) = factory {
                let creates = matches!(base, "newtcolorbox" | "NewTColorBox" | "ProvideTColorBox");
                if !creates || !declared_environments.contains(&declaration.name) {
                    selected.insert(declaration.name.clone(), declaration.arguments);
                    declared_environments.insert(declaration.name);
                }
                cursor = declaration.end;
                continue;
            }
            if depth == 0
                && range.is_none()
                && !command.ends_with('*')
                && !config.syntax.commands.contains_key(base)
                && !redefined_commands.contains(base)
                && let Some((target, finish)) = box_factory_shape(source, base, end)
            {
                if base != "ProvideTColorBox" || !declared_environments.contains(&target) {
                    selected.remove(&target);
                    declared_environments.insert(target);
                }
                cursor = finish;
                continue;
            }
            if (range.is_none()
                || range.is_some_and(|range| range.start == cursor)
                    && matches!(base, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet"))
                && (macro_declaration(base)
                    || matches!(base, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet"))
                && let Some((target, after_target)) = definition_target(source, base, end)
            {
                if base.contains("environment") || base.contains("Environment") {
                    selected.remove(&target);
                    declared_environments.insert(target);
                } else {
                    redefined_commands.insert(target);
                }
                if let Some(end) = definition_end(source, base, after_target) {
                    cursor = end;
                    continue;
                }
            }
            if let Some(range) = range {
                cursor = range.end;
            } else if base == "string" {
                let next = trivia_end(source, end);
                // The single-name scanner returns None on an incomplete string
                // token, discarding any earlier selection for every name.
                let Some(character) = source[next..].chars().next() else {
                    return HashMap::new();
                };
                cursor = crate::syntax::command_at(source, next)
                    .map_or(next + character.len_utf8(), |(_, end)| end);
            } else {
                cursor = end;
            }
            continue;
        }
        if let Some(range) = range {
            cursor = range.end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => (),
        }
        cursor += source[cursor..]
            .chars()
            .next()
            .expect("cursor before source end")
            .len_utf8();
    }
    selected.retain(|name, _| candidates.contains(name));
    selected
}

/// Parse complete literal factory calls. The caller excludes protected source
/// and locally redefined factory commands before applying the result.
pub(crate) fn verbatim_declaration(
    source: &str,
    command: &str,
    end: usize,
) -> Option<VerbatimDeclaration> {
    let mut cursor = end;
    let mut names = Vec::new();
    let mut terminator = VerbatimTerminator::AnyPosition;
    match command {
        "NewPitonEnvironment"
        | "RenewPitonEnvironment"
        | "DeclarePitonEnvironment"
        | "ProvidePitonEnvironment" => {
            names.push(literal_name(&source[mandatory(source, &mut cursor)?])?);
            mandatory(source, &mut cursor)?;
            mandatory(source, &mut cursor)?;
            mandatory(source, &mut cursor)?;
        }
        // minted.sty:1895--1910; the starred environment takes extra options.
        "newminted" => {
            let explicit = option(source, &mut cursor);
            let language = mandatory(source, &mut cursor)?;
            mandatory(source, &mut cursor)?;
            let name = if let Some(explicit) = explicit {
                let value = source[explicit].trim();
                if value.is_empty() {
                    format!("{}code", literal_name(&source[language])?)
                } else {
                    literal_name(value)?
                }
            } else {
                format!("{}code", literal_name(&source[language])?)
            };
            names.extend([name.clone(), format!("{name}*")]);
        }
        // listings.sty:2194--2228.
        "lstnewenvironment" => {
            names.push(literal_name(&source[mandatory(source, &mut cursor)?])?);
            if let Some(count) = option(source, &mut cursor) {
                source[count].trim().parse::<u8>().ok()?;
                option(source, &mut cursor);
            }
            mandatory(source, &mut cursor)?;
            mandatory(source, &mut cursor)?;
        }
        // tcblistingscore.code.tex:298--359.
        "newtcblisting" | "renewtcblisting" => {
            option(source, &mut cursor);
            names.push(literal_name(&source[mandatory(source, &mut cursor)?])?);
            if let Some(count) = option(source, &mut cursor) {
                source[count].trim().parse::<u8>().ok()?;
                option(source, &mut cursor);
            }
            mandatory(source, &mut cursor)?;
        }
        "NewTCBListing" | "RenewTCBListing" | "DeclareTCBListing" | "ProvideTCBListing" => {
            option(source, &mut cursor);
            names.push(literal_name(&source[mandatory(source, &mut cursor)?])?);
            mandatory(source, &mut cursor)?;
            mandatory(source, &mut cursor)?;
        }
        // comment.sty:234--276. Included content also passes through a file.
        "excludecomment" | "includecomment" | "specialcomment" | "processcomment" => {
            names.push(literal_name(&source[mandatory(source, &mut cursor)?])?);
            let arguments = match command {
                "specialcomment" => 2,
                "processcomment" => 3,
                _ => 0,
            };
            for _ in 0..arguments {
                mandatory(source, &mut cursor)?;
            }
            terminator = VerbatimTerminator::UnindentedLine;
        }
        _ => return crate::code_package_syntax::source_declaration(source, command, end),
    }
    Some(VerbatimDeclaration {
        names,
        end: cursor,
        terminator,
    })
}

pub(crate) fn is_source_verbatim_environment_outside(
    source: &str,
    name: &str,
    protected: &[Range<usize>],
) -> bool {
    if !matches!(name, "listing" | "listing*") {
        return false;
    }
    let mut cursor = 0;
    let mut depth = 0usize;
    let mut disabled = 0usize;
    let mut declared_literals = Vec::new();
    while cursor < source.len() {
        let index = protected.partition_point(|range| range.end <= cursor);
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            cursor = range.end.min(source.len());
            continue;
        }
        match source.as_bytes()[cursor] {
            b'%' => {
                let end = source[cursor..]
                    .find('\n')
                    .map_or(source.len(), |n| cursor + n + 1);
                let comment = source[cursor + 1..end].trim();
                if matches!(comment, "tift: off" | "tex-fmt: off") {
                    disabled += 1;
                } else if matches!(comment, "tift: on" | "tex-fmt: on") {
                    disabled = disabled.saturating_sub(1);
                }
                cursor = end;
                continue;
            }
            _ if disabled > 0 => {
                cursor = source[cursor..]
                    .find('%')
                    .map_or(source.len(), |n| cursor + n);
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'\\' => {
                let Some((command, end)) = crate::syntax::command_at(source, cursor) else {
                    return false;
                };
                if command == "iffalse" {
                    cursor = false_branch_end(source, end);
                    continue;
                }
                if depth == 0
                    && let Some(declaration) = verbatim_declaration(source, command, end)
                {
                    declared_literals.extend(declaration.names);
                    cursor = declaration.end;
                    continue;
                }
                if matches!(command, "verb" | "verb*")
                    && let Some(delimiter) = source[end..].chars().next()
                {
                    let body = end + delimiter.len_utf8();
                    cursor = source[body..]
                        .find(delimiter)
                        .map_or(source.len(), |n| body + n + delimiter.len_utf8());
                    continue;
                }
                if matches!(
                    command,
                    "lstinline" | "lstinline*" | "mintinline" | "mintinline*"
                ) {
                    let mut after = end;
                    option(source, &mut after);
                    if command.starts_with("mintinline") && mandatory(source, &mut after).is_none()
                    {
                        return false;
                    }
                    after = trivia_end(source, after);
                    if source.as_bytes().get(after) == Some(&b'{') {
                        if mandatory(source, &mut after).is_some() {
                            cursor = after;
                            continue;
                        }
                    } else if let Some(delimiter) = source[after..].chars().next() {
                        let body = after + delimiter.len_utf8();
                        cursor = source[body..]
                            .find(delimiter)
                            .map_or(source.len(), |n| body + n + delimiter.len_utf8());
                        continue;
                    }
                }
                if command == "string" {
                    let after = trivia_end(source, end);
                    cursor = crate::syntax::command_at(source, after)
                        .map_or(after.saturating_add(1).min(source.len()), |(_, end)| end);
                    continue;
                }
                if command == "begin" {
                    let mut after = end;
                    if let Some(environment) = mandatory(source, &mut after) {
                        let environment = &source[environment];
                        if matches!(
                            environment,
                            "verbatim"
                                | "verbatim*"
                                | "Verbatim"
                                | "Verbatim*"
                                | "BVerbatim"
                                | "LVerbatim"
                                | "minted"
                                | "lstlisting"
                                | "comment"
                        ) || ADDITIONAL_VERBATIM_ENVIRONMENTS.contains(&environment)
                            || declared_literals.iter().any(|name| name == environment)
                        {
                            let terminator = format!("\\end{{{environment}}}");
                            cursor = source[after..]
                                .find(&terminator)
                                .map_or(source.len(), |n| after + n + terminator.len());
                            continue;
                        }
                    }
                }
                if depth == 0
                    && matches!(
                        command,
                        "usepackage" | "RequirePackage" | "RequirePackageWithOptions"
                    )
                {
                    let mut after = end;
                    if command != "RequirePackageWithOptions" {
                        option(source, &mut after);
                    }
                    if let Some(packages) = mandatory(source, &mut after) {
                        if source[packages]
                            .split(',')
                            .any(|package| package.trim() == "moreverb")
                        {
                            return true;
                        }
                        cursor = after;
                        continue;
                    }
                }
                cursor = end;
                continue;
            }
            _ => (),
        }
        cursor += source[cursor..]
            .chars()
            .next()
            .expect("valid cursor")
            .len_utf8();
    }
    false
}

fn false_branch_end(source: &str, mut cursor: usize) -> usize {
    let mut depth = 1usize;
    while cursor < source.len() {
        if source.as_bytes()[cursor] == b'%' {
            cursor = source[cursor..]
                .find('\n')
                .map_or(source.len(), |n| cursor + n + 1);
            continue;
        }
        if let Some((command, end)) = crate::syntax::command_at(source, cursor) {
            if matches!(
                command,
                "if" | "ifcat"
                    | "ifnum"
                    | "ifdim"
                    | "ifodd"
                    | "ifvmode"
                    | "ifhmode"
                    | "ifmmode"
                    | "ifinner"
                    | "ifvoid"
                    | "ifhbox"
                    | "ifvbox"
                    | "ifx"
                    | "ifeof"
                    | "iftrue"
                    | "iffalse"
                    | "ifcase"
                    | "ifdefined"
                    | "ifcsname"
                    | "iffontchar"
            ) {
                depth += 1;
            } else if command == "fi" {
                depth -= 1;
                if depth == 0 {
                    return end;
                }
            } else if command == "else" && depth == 1 {
                return end;
            }
            cursor = end;
            continue;
        }
        cursor += source[cursor..]
            .chars()
            .next()
            .expect("valid cursor")
            .len_utf8();
    }
    source.len()
}
