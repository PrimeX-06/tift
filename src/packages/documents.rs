//! Literal environment and command grammar from exam-zh and ltx-talk.
//! Metadata applies to matching syntax independently of document-class loading.

use crate::config::CommandRule;

pub(crate) const LIST_ENVIRONMENTS: &[&str] = &[
    // exam-zh notice wraps enumerate; choices and select split collected bodies
    // at item/item* and sitem/sitem*, respectively.
    "notice",
    "choices",
    "select",
    "calculations",
    "multifigures",
];

pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &["frame*"];

/// Overlay <...> is `d`; square-bracket options are `o`; required groups are `m`.
pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // exam-zh-{question,choices,chinese-english}.sty, installed 2026 release.
        "question" | "problem" | "solution" | "choices" | "select" | "lineto" | "material"
        | "poem" | "writingbox" => "o",
        // notice: O{}O{}; calculations and multifigures: O{}+O{}.
        "notice" | "calculations" | "multifigures" => "oo",
        // ltx-talk.cls frame, column, block and overlay declarations.
        // The frame-title-arg setting adds a required title; explicit configured
        // signature "dom" disambiguates that form from a braced first body token.
        "frame" => "do",
        "columns" => "do",
        "column" => "do",
        "block" => "dm",
        "actionenv" | "alertenv" | "onlyenv" | "invisibleenv" | "uncoverenv" | "visibleenv" => "d",
        "overlayarea" => "mm",
        "overprint" => "o",
        // Overlay-capable standard paragraph/list and float environments.
        "quote" | "quotation" | "verse" => "do",
        "itemize" | "enumerate" | "description" => "o",
        "figure" | "table" => "do",
        _ => return None,
    })
}

pub(crate) fn is_item(name: &str, environment: &str) -> bool {
    matches!(
        (name.trim_end_matches('*'), environment),
        ("sitem", "select")
            | (
                "item",
                "notice" | "choices" | "calculations" | "multifigures"
            )
    )
}

// One declaration generates both the fast signature match and command registry.
// New command names therefore cannot be omitted from the argument-rule table.
macro_rules! command_signatures {
    ($( $($name:literal)|+ => $signature:literal ),+ $(,)?) => {
        /// Exact leading grammar, including overlays and optional key lists.
        pub(crate) fn command_signature(name: &str) -> Option<&'static str> {
            Some(match name.trim_end_matches('*') {
                $( $($name)|+ => $signature, )+
                _ => return None,
            })
        }

        pub(crate) fn command_rule(name: &str) -> Option<&'static CommandRule> {
            static COMMANDS: std::sync::OnceLock<std::collections::BTreeMap<String, CommandRule>> =
                std::sync::OnceLock::new();
            COMMANDS.get_or_init(|| {
                let mut commands = std::collections::BTreeMap::new();
                $(
                    let arguments = $signature.bytes().filter(|&slot| slot == b'm').count();
                    let optional_arguments = $signature.contains('o');
                    for name in [$($name),+] {
                        commands.insert(name.into(), CommandRule {
                            arguments: Some(arguments),
                            optional_arguments,
                            ..Default::default()
                        });
                    }
                )+
                commands
            }).get(name.trim_end_matches('*'))
        }
    };
}

command_signatures! {
    // exam-zh public configuration, printable answers and text utilities.
    "examsetup" | "questionsetup" | "fillinsetup" | "setchoices" => "m",
    "AddChoicesCounter" | "AddCalculationsCounter" | "AddQuestionCounter" | "AddFillinCounter" => "mm",
    "ExamPrintAnswerSet" | "information" | "warning" | "subject" | "score" | "examsquare"
        | "linelistset" | "lineconnect" | "zhu" => "om",
    "textfigure" => "omm",
    "circlednumber" | "tikzcirclednumber" => "m",
    "paren" | "secret" | "draftpaper" => "o",
    // One bracket contains the answer; two provide keys then answer.
    "fillin" => "oo",
    "ExamPrintAnswer" | "scoringbox" => "",
    // ltx-talk overlay commands (D<>/d<> plus optional metadata).
    "frametitle" | "framesubtitle" | "reuseframe" | "footnote" => "dom",
    "section" | "subsection" | "subsubsection" => "dom",
    "includegraphics" => "doom",
    "label" | "textbf" | "textit" | "textmd" | "textnormal" | "textrm" | "textsc"
        | "textsf" | "textsl" | "texttt" | "textup" | "emph" => "dm",
    "color" | "pagecolor" => "dom",
    "mathcolor" | "textcolor" => "domm",
    "item" => "do",
    "alert" | "only" | "invisible" | "uncover" | "visible" | "action" | "mode" => "dm",
    "alt" => "dmm",
    "temporal" => "dmmm",
    "onslide" | "appendix" => "d",
    "pause" | "tableofcontents" | "maketitle" => "o",
    "institute" | "subtitle" | "author" | "date" | "title" => "om",
    "appendixmarker" => "",
    // Declaration exposes fixed arguments with optional model data.
    "DeclareColor" => "mom",
}
