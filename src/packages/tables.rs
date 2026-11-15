//! Literal headers of common table environments.
//!
//! Signatures describe arguments before the table body: `o` skips ordinary
//! whitespace before a bracket option, `O` requires token adjacency, and `m`
//! consumes one mandatory TeX argument. Environment names are exact.

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &[
    "tabular",
    "tabular*",
    "array",
    "+array",
    "tabularx",
    "tabulary",
    "longtable",
    "xltabular",
    "xtabular",
    "xtabular*",
    "mpxtabular",
    "mpxtabular*",
    "supertabular",
    "supertabular*",
    "mpsupertabular",
    "mpsupertabular*",
    "tblr",
    "longtblr",
    "talltblr",
    "booktabs",
    "longtabs",
    "talltabs",
    "NiceTabular",
    "NiceTabularX",
    "NiceTabular*",
];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // array.sty: \@array[#1]#2; longtable.sty: \LT@array[#1]#2.
        // xtab/supertabular likewise read a placement option before columns.
        "tabular" | "array" | "longtable" | "xtabular" | "mpxtabular" | "supertabular"
        | "mpsupertabular" => "om",
        // Width precedes the placement option and column preamble.
        "tabular*" | "tabularx" | "tabulary" | "xtabular*" | "mpxtabular*" | "supertabular*"
        | "mpsupertabular*" => "mom",
        // xltabular consumes placement, then forwards width/columns to tabularx.
        "xltabular" => "omm",
        // tabularray's NewTblrEnviron uses { O{} m +b }. The mandatory inner
        // options include colspec; brackets after them belong to the first cell.
        "tblr" | "longtblr" | "talltblr" | "booktabs" | "longtabs" | "talltabs" => "om",
        // tabularray's amsmath library uses the same inner-option header.
        "+array" => "om",
        // nicematrix's trailing !O option is tight; spaced brackets are data.
        "NiceTabular" => "omO",
        "NiceTabularX" | "NiceTabular*" => "momO",
        _ => return None,
    })
}

pub(crate) fn is_table(name: &str) -> bool {
    header_signature(name).is_some()
}

/// Tables whose surrounding paragraph may require adjacent box boundaries.
/// The mathematical array has its own math-context boundary handling.
pub(crate) fn is_box(name: &str) -> bool {
    is_table(name) && !matches!(name, "array" | "+array")
}

/// Zero-based position in the header signature of the column preamble.
/// For tabularray this is the entire inner-option argument containing colspec.
/// In these table signatures it is always the last mandatory argument.
pub(crate) fn column_spec_slot(name: &str) -> Option<usize> {
    header_signature(name)?.rfind('m')
}


