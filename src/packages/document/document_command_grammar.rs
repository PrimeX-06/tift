//! Literal environment and command grammar from exam, exam-zh and ltx-talk.
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
    "questions",
    "parts",
    "subparts",
    "subsubparts",
    "oneparchoices",
    "checkboxes",
    "oneparcheckboxes",
];

pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &["frame*"];

/// scrguide.cls chooses one of these two document variants. A common
/// literal environment opening has the same stack effect in either edition.
pub(crate) fn branch_environment_boundary(
    source: &str,
    name: &str,
    end: usize,
) -> Option<(crate::config::EnvironmentAlias, usize)> {
    if name != "iffree" {
        return None;
    }
    let arguments =
        crate::layout::signature_argument_ranges(source, end..source.len(), "mm".bytes());
    if arguments.len() != 2 {
        return None;
    }
    let mut common = None;
    for argument in &arguments {
        if source.as_bytes()[argument.start] != b'{' {
            return None;
        }
        let text = &source[argument.start + 1..argument.end - 1];
        let start = crate::trivia::skip(text, 0);
        let (command, end) = crate::syntax::command_at(text, start)?;
        let (environment, true, mut finish) = crate::syntax::env_after_command(text, command, end)?
        else {
            return None;
        };
        finish = crate::trivia::skip(text, finish);
        if text.as_bytes().get(finish) == Some(&b'[') {
            finish =
                crate::trivia::skip(text, crate::syntax::balanced_end(text, finish, b'[', b']')?);
        }
        if finish != text.len() || common.as_ref().is_some_and(|value| value != environment) {
            return None;
        }
        common = Some(environment.to_owned());
    }
    Some((
        crate::config::EnvironmentAlias {
            environment: common?,
            begin: true,
        },
        arguments[1].end,
    ))
}

pub(crate) fn documentation_short_verbatim(class: Option<&str>) -> Option<&'static str> {
    // ltxdoc.cls:72 activates the doc package's short verbatim bar for its
    // documentation. The delimiter is scoped to that documentation view.
    (class == Some("ltxdoc")).then_some("|")
}

/// Overlay <...> is `d`; square-bracket options are `o`; required groups are `m`.
pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // l3doc: a variant/options list followed by control-sequence names.
        "function" | "variable" | "codedescribe" => "om",
        // exam-zh-{question,choices,chinese-english}.sty, installed 2026 release.
        "question" | "problem" | "solution" | "choices" | "select" | "lineto" | "material"
        | "poem" | "writingbox" => "o",
        // notice: O{}O{}; calculations and multifigures: O{}+O{}.
        "notice" | "calculations" | "multifigures" => "oo",
        // exam.cls:2917,3263,3385,3401,4199--4395,7711--8066.
        "questions" | "parts" | "subparts" | "subsubparts" | "oneparchoices" | "checkboxes"
        | "oneparcheckboxes" | "coverpages" => "",
        "solutionorbox" | "solutionorlines" | "solutionordottedlines" | "solutionorgrid" => "o",
        "solutionbox" => "m",
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
    exam_item(name).is_some_and(|(scopes, _)| scopes.contains(&environment))
        || matches!(
            (name.trim_end_matches('*'), environment),
            ("sitem", "select")
                | (
                    "item",
                    "notice" | "choices" | "calculations" | "multifigures"
                )
        )
}

fn exam_item(name: &str) -> Option<(&'static [&'static str], &'static str)> {
    Some(match name {
        "question" | "bonusquestion" => (&["questions"], "o"),
        "part" | "bonuspart" => (&["parts"], "o"),
        "subpart" | "bonussubpart" => (&["subparts"], "o"),
        "subsubpart" | "bonussubsubpart" => (&["subsubparts"], "o"),
        "titledquestion" | "bonustitledquestion" => (&["questions"], "mo"),
        "choice" | "CorrectChoice" | "correctchoice" => (
            &["choices", "oneparchoices", "checkboxes", "oneparcheckboxes"],
            "",
        ),
        _ => return None,
    })
}

pub(crate) fn context_command_signature(
    _: &str,
    name: &str,
    _: usize,
    context: &crate::packages::InvocationContext<'_>,
) -> Option<&'static str> {
    let (scopes, shape) = exam_item(name)?;
    scopes.contains(&context.environment()?).then_some(shape)
}

/// File readers end with a filename. Earlier arguments (including graphics
/// keys and listings captions) remain available to their own formatters.
pub(crate) fn is_file_reader(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "includegraphics"
            | "verbatiminput"
            | "VerbatimInput"
            | "BVerbatimInput"
            | "LVerbatimInput"
            | "lstinputlisting"
            | "LTXtable"
    )
}

pub(crate) fn filename_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<std::ops::Range<usize>>> {
    if !is_file_reader(name) {
        return None;
    }
    let shape = if name.trim_end_matches('*') == "includegraphics" {
        "s d<> s o o m"
    } else {
        command_signature(name)?
    };
    let signature = crate::syntax::xparse::Signature::parse(shape)?;
    let cursor = signature.invocation_cursor(source, cursor);
    let file = signature.slot_ranges(source, cursor..source.len())?.pop()?;
    Some(std::iter::once(file.body).collect())
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
    "verbatiminput" => "sm",
    "VerbatimInput" | "BVerbatimInput" | "LVerbatimInput" => "som",
    "lstinputlisting" => "om",
    "LTXtable" => "mm",
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
    // beamer-reveal.sty:506: overlays, keys, coordinates, optional \at and URL.
    "iframe" => "D<>{1-}O{}d()t\\at d()m",
    // beamerbasecolor.sty:72 returns first/second-of-two after the color name.
    "ifbeamercolorempty" => "ommm",
}
