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
    // attachfile/embedfile commands keep the filename literal while leaving
    // the visible text of textattachfile available to the formatter.
    "attachfile" | "embedfile" => ("om", "o"),
    "textattachfile" => ("omm", "om"),
    "attachfilesetup" | "embedfilesetup" => ("m", "m"),
    // endnotes.sty: optional note marks precede the note text. addtoendnotes
    // stores a delayed command in the auxiliary endnotes stream.
    "endnote" => ("om", "o"),
    "endnotemark" => ("o", "o"),
    "endnotetext" => ("om", "o"),
    "addtoendnotes" => ("m", "m"),
    "theendnotes" => ("", ""),
    // perpage.sty: optional initial counter value, then the counter name.
    "MakePerPage" | "MakeSortedPerPage" => ("om", "om"),
    "MakeSorted" => ("m", "m"),
    // parallel.sty passes each language column as a printable text argument.
    "ParallelLText" | "ParallelRText" => ("m", ""),
    "ParallelPar" | "colplacechunks" => ("", ""),
    // parcolumns.sty selects an optional target column for each text chunk.
    "colchunk" => ("om", "o"),
    // etoc.sty stores display hooks and line-style templates for later TOCs.
    "etocsettocstyle" | "etocsetlevel" => ("mm", "mm"),
    "etocsetnexttocdepth" => ("m", "m"),
    "etocsetstyle" => ("mmmmm", "mmmmm"),
    "localtableofcontents" | "locallistoffigures" | "locallistoftables" => ("", ""),
    // tocloft.sty entry names and dimension values are layout metadata.
    "cftsetindents" => ("mmm", "mmm"),
    "cftsetpnumwidth" | "cftsetrmarg" => ("m", "m"),
    // pagenote.sty writes notes to a document-end note list.
    "pagenote" => ("m", ""),
    "printnotes" => ("", ""),
    // parnotes.sty formats paragraph notes after the paragraph boundary.
    "parnote" => ("m", ""),
    "parnotes" => ("", ""),
    // shorttoc.sty prints abbreviated tables with an explicit title and depth.
    "shorttoc" | "shorttableofcontents" => ("mm", "mm"),
    // authblk.sty affiliations use a marker and a printable institution line.
    "affil" => ("om", "o"),
    "Authfont" | "Affilfont" => ("m", "m"),
    // endfloat.sty delays float environments named by their literal type.
    "DeclareDelayedFloatFlavor" => ("mm", "mm"),
    "processdelayedfloats" => ("", ""),
    // quotchap.sty stores quotes for the next chapter heading.
    "qauthor" => ("m", ""),
    "qsetcnfont" => ("m", "m"),
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
