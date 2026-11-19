//! Exercise setup, project paths and calendar data.

use super::{BTreeMap, CommandRule};

commands! {
    // import.sty:44--74. The optional star may also be separated by spaces.
    "import" | "subimport" | "inputfrom" | "subinputfrom" | "includefrom"
        | "subincludefrom" => ("mm", "mm"),
    // subfiles.sty:118--134 delegates to import and resolves a literal path.
    "subfile" | "subfileinclude" | "subfix" => ("m", "m"),
    // xsim.sty:4635--4656,5186--5193,5265--5330.
    "xsimsetup" | "collectexercises" | "collectexercisesstop" | "loadxsimstyle" => ("m", "m"),
    "DeclareExerciseEnvironmentTemplate" => ("mmm", "mmm"),
    "DeclareExerciseType" | "DeclareExerciseHeadingTemplate" | "DeclareExerciseTableTemplate"
        | "UseExerciseTemplate" | "DeclareExercisePropertyAlias" => ("mm", "mm"),
    "DeclareExerciseTagging" => ("m", "m"),
    "DeclareExerciseCollection" | "printcollection" | "printsolutionstype" => ("om", "om"),
    "printexercise" | "printsolution" => ("omm", "omm"),
    "printsolutions" | "printallsolutions" => ("o", "o"),
    "xsimstyle" => ("mo", "mo"),
    // datetime2.sty:219,491--528,642--695,784,834--955,1606--1773.
    "DTMsetup" | "DTMdate" | "DTMDate" | "DTMtime" | "DTMuse" | "DTMusedate" | "DTMsetstyle"
        | "DTMsetdatestyle" | "DTMsettimestyle" | "DTMsetzonestyle" => ("m", "m"),
    "DTMsetregional" => ("o", "o"),
    "DTMsavedate" | "DTMsavetime" | "DTMsavetimestamp" | "DTMnewdatestyle"
        | "DTMnewtimestyle" | "DTMnewzonestyle" => ("mm", "mm"),
    "DTMdisplaytime" => ("mmm", "mmm"),
    "DTMdisplaydate" | "DTMDisplaydate" => ("mmmm", "mmmm"),
    "DTMnewstyle" => ("mmmmm", "mmmmm"),
}

/// The lexer already consumes an attached star as part of the control name.
pub(super) fn separated_star_shape(name: &str) -> Option<&'static str> {
    Some(match name {
        "import" | "subimport" | "inputfrom" | "subinputfrom" | "includefrom"
        | "subincludefrom" => "mmm",
        "printsolutions" | "printallsolutions" => "mo",
        "printsolutionstype" => "mom",
        "xsimstyle" => "mmo",
        "DTMdate" | "DTMDate" => "mm",
        _ => return None,
    })
}
