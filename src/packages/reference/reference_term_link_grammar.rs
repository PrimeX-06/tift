//! Reference labels, glossary records and abbreviation interfaces.
//!
//! The signature describes the complete call; the bit mask describes only its
//! literal operands. Printed text remains available to ordinary TeX layout.

use crate::{config::CommandRule, trivia};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modifier {
    None,
    Star,
    SimpleStar,
    /// glossaries' hyperlink switch accepts either a star or a plus.
    Hyper,
}

#[derive(Clone, Copy)]
struct Command {
    shape: &'static str,
    data: u16,
    modifier: Modifier,
}

macro_rules! commands {
    ($( $($name:literal)|+ => ($shape:literal, $data:expr, $modifier:ident) ),+ $(,)?) => {
        fn fixed_command(name: &str) -> Option<Command> {
            Some(match crate::syntax::unstarred(name) {
                $( $($name)|+ => Command {
                    shape: $shape, data: $data, modifier: Modifier::$modifier,
                }, )+
                _ => return None,
            })
        }

        pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
            $(
                for name in [$($name),+] {
                    commands.entry(name.into()).or_insert(CommandRule {
                        arguments: Some($shape.bytes().filter(|&slot| slot == b'm').count()),
                        optional_arguments: $shape.contains('o'),
                        ..Default::default()
                    });
                }
            )+
        }
    };
}

commands! {
    // tools/varioref.sty:738--750,850--861,905--954. Optional arguments
    // are printed phrases; the following label operands are literal data.
    "vref" | "Vref" => ("om", 0b10, Star),
    "vpageref" => ("oom", 0b100, Star),
    "vpagerefrange" => ("omm", 0b110, Star),
    "vrefrange" => ("omm", 0b110, None),
    "fullref" => ("m", 0b1, None),
    "vrefpagenum" => ("mm", 0b11, None),
    "vpagerefcompare" => ("mmmm", 0b11, None),
    "vpagerefnearby" => ("mmm", 0b1, None),
    "reftextvario" => ("mm", 0, None),
    // zref/zref-user.sty:59--95; zref-base.sty:459,716. zkvlabel reads
    // a property list first and a label second, both through the wrapper.
    "zlabel" | "zpageref" | "zrefused" => ("m", 0b1, None),
    "zkvlabel" => ("mm", 0b11, None),
    "zref" => ("om", 0b11, None),
    // zref-clever.sty:793--810,2654--2664,2920,3460,3508. Label lists and
    // settings are data even when they contain deferred LaTeX tokens.
    "zcref" | "zcpageref" => ("om", 0b11, Star),
    "zcsetup" => ("m", 0b1, None),
    "zcDeclareLanguage" => ("om", 0b11, None),
    "zcDeclareLanguageAlias" | "zcLanguageSetup" | "zcRefTypeSetup" => ("mm", 0b11, None),
    // xr-hyper 2026 delegates to tools/xr.sty:43--48. The final URL is
    // optional after the mandatory auxiliary-file name.
    "externaldocument" | "externalcitedocument" => ("oomo", 0b1111, None),
    // glossaries/base/glossaries.sty:2307--2344,2627,3000,3592,
    // 3782--3813,4363--4496,4803--5038,5323--5354,6524,6581,8057.
    "gls" | "Gls" | "GLS" | "glspl" | "Glspl" | "GLSpl"
        | "acrshort" | "Acrshort" | "ACRshort"
        | "acrshortpl" | "Acrshortpl" | "ACRshortpl"
        | "acrlong" | "Acrlong" | "ACRlong"
        | "acrlongpl" | "Acrlongpl" | "ACRlongpl"
        | "acrfull" | "Acrfull" | "ACRfull"
        | "acrfullpl" | "Acrfullpl" | "ACRfullpl" => ("omo", 0b11, Hyper),
    "glslink" | "Glslink" | "glsdisp" | "Glsdisp" => ("omm", 0b11, Hyper),
    "glshyperlink" => ("om", 0b10, None),
    "glssee" => ("omm", 0b110, None),
    "newglossaryentry" | "provideglossaryentry" => ("mm", 0b11, None),
    "longnewglossaryentry" | "longprovideglossaryentry" => ("mmm", 0b11, Star),
    "newglossary" => ("ommmm", 0b1111, Star),
    "altnewglossary" => ("mmm", 0b11, None),
    "newacronym" | "newabbreviation" => ("ommm", 0b11, None),
    "loadglsentries" | "glsadd" => ("om", 0b11, None),
    "printglossary" | "printnoidxglossary" | "printacronyms"
        | "glsaddall" | "glsaddallunused" | "glsresetall" | "glsunsetall" => ("o", 0b1, None),
    "printglossaries" | "printnoidxglossaries" | "makeglossaries" | "makenoidxglossaries" => ("", 0, None),
    "glsreset" | "glsunset" | "glsentryname" | "Glsentryname"
        | "glsentrydesc" | "Glsentrydesc" | "glsentrydescplural"
        | "glsentrytext" | "Glsentrytext" | "glsentryplural" | "Glsentryplural"
        | "glsentrysymbol" | "glsentrysymbolplural"
        | "glsentryfirst" | "Glsentryfirst" | "glsentryfirstplural" | "Glsentryfirstplural"
        | "glsentrytype" | "glsentrysort" | "glsentryparent"
        | "glsentryshort" | "Glsentryshort" | "glsentryshortpl" | "Glsentryshortpl"
        | "glsentrylong" | "Glsentrylong" | "glsentrylongpl" | "Glsentrylongpl"
        | "glsentryfull" | "Glsentryfull" | "glsentryfullpl" | "Glsentryfullpl" => ("m", 0b1, None),
    "ifglsentryexists" | "ifglsused" => ("mmm", 0b1, None),
    "setglossarypreamble" => ("om", 0b1, None),
    "setglossarystyle" | "setacronymstyle" => ("m", 0b1, None),
    // glossaries.sty:8229--8250 stores both display/style replacement bodies.
    "newacronymstyle" | "renewacronymstyle" => ("mmm", 0b111, None),
    // glossaries-extra.sty:7081,9858--9883,10986,11253--11669,11873.
    // These use the same options/label/printed-insertion order as gls.
    "glsxtrshort" | "Glsxtrshort" | "GLSxtrshort"
        | "glsxtrshortpl" | "Glsxtrshortpl" | "GLSxtrshortpl"
        | "glsxtrlong" | "Glsxtrlong" | "GLSxtrlong"
        | "glsxtrlongpl" | "Glsxtrlongpl" | "GLSxtrlongpl"
        | "glsxtrfull" | "Glsxtrfull" | "GLSxtrfull"
        | "glsxtrfullpl" | "Glsxtrfullpl" | "GLSxtrfullpl" => ("omo", 0b11, Hyper),
    "setabbreviationstyle" | "glsxtrresourcefile" => ("om", 0b11, None),
    "glssetcategoryattribute" | "glssetcategoryattributes" => ("mmm", 0b111, None),
    "glsunsetcategoryattribute" => ("mm", 0b11, None),
    // acro.sty:676--819,3317--3384,5224--5289. The public acro commands
    // accept star/options before the ID; acronym uses the same order where
    // its optional line-breaking penalty is supported (acronym.sty:568--747).
    "ac" | "Ac" | "acp" | "Acp" | "iac" | "Iac"
        | "Acs" | "Acsp" | "iacs" | "Iacs" | "iacl" | "Iacl"
        | "aca" | "Aca" | "acap" | "Acap" | "iaca" | "Iaca"
        | "acf" | "Acf" | "acfp" | "Acfp" | "iacf" | "Iacf"
        | "acfi" | "Acfi" | "acfip" | "Acfip" | "acshow" => ("om", 0b11, Star),
    // acronym's short/long-only forms have no optional penalty. Their shared
    // minimum remains one ID; a certain acro load adds its options below.
    "acs" | "acsp" | "acl" | "Acl" | "aclp" | "Aclp" => ("m", 0b1, Star),
    "DeclareAcronym" | "NewAcroPreset" | "RenewAcroPreset" | "DeclareAcroPreset" => ("mm", 0b11, None),
    "acsetup" | "acroloadstyle" | "acuse" | "acreset" | "acrotranslate" => ("m", 0b1, None),
    "acuseall" | "acresetall" | "acbarrier" | "acswitchoff" | "acswitchon" => ("", 0, None),
    "acroifusedTF" => ("mmm", 0b1, None),
    "acroifusedT" | "acroifusedF" => ("mm", 0b1, None),
    "NewAcroCommand" | "RenewAcroCommand" | "DeclareAcroCommand" | "ProvideAcroCommand" => ("mmm", 0b111, None),
    "NewAcroTemplate" | "RenewAcroTemplate" | "SetupAcroTemplate" | "SetupNextAcroTemplate" => ("omm", 0b111, None),
    "UseAcroTemplate" => ("omo", 0b111, None),
    // acronym.sty:224--235,252--295,348--404,763--777. ID, optional
    // short form, long form: the two display operands remain readable.
    "newacro" | "acrodef" | "acro" | "newacroplural" | "acrodefplural" | "acroplural" => ("mom", 0b1, None),
    "newacroindefinite" | "acrodefindefinite" | "acroindefinite" => ("mmm", 0b1, None),
    "acused" | "acronymused" => ("m", 0b1, None),
    "acsu" | "aclu" | "Aclu" => ("m", 0b1, Star),
    "acroextra" => ("m", 0, None),
    // prettyref.sty:21--23,30; fancyref.sty:289--304,488--507.
    // Format bodies contain deferred parameter tokens and keep their bytes.
    "prettyref" => ("m", 0b1, None),
    "newrefformat" | "fancyrefchangeprefix" => ("mm", 0b11, None),
    "fref" | "Fref" => ("om", 0b11, None),
    "frefformat" | "Frefformat" => ("mmm", 0b111, None),
    // bibunits.sty:225--260,307--324. bibliographyunit has only an
    // optional division command; putbib accepts an optional database list.
    "defaultbibliography" | "defaultbibliographystyle" => ("m", 0b1, None),
    "bibliographyunit" | "putbib" => ("o", 0b1, None),
    // cite.sty: number-only citation aliases keep their optional note separate
    // from the literal citation-key list.
    "citen" | "citenum" | "citeonline" => ("om", 0b10, None),
    // natbib: alias definitions have one key followed by printable alias text.
    "defcitealias" => ("mm", 0b01, None),
    "citetalias" | "citepalias" => ("oom", 0b100, None),
    // natbib also exposes the same alias commands under biblatex's
    // natbib-compatibility module.
    "bibpunct" => ("ommmmmm", 0b1111111, None),
    "setcitestyle" => ("m", 0b1, None),
    // bibentry.sty: bibliography entries are referenced by one key; the
    // bibliography file list is delayed metadata.
    "bibentry" => ("m", 0b1, None),
    "nobibliography" => ("m", 0b1, None),
    // nomencl.sty: the optional sort prefix and symbol are data, while the
    // description remains available for ordinary text and nested math.
    "nomenclature" => ("omm", 0b011, None),
    "nomgroup" => ("m", 0b1, None),
    "makenomenclature" => ("", 0, None),
    "printnomenclature" => ("o", 0b1, None),
    // multibib and chapterbib declarations and file inputs.
    "newcites" => ("mm", 0b11, None),
    "cbinput" => ("m", 0b1, None),
    // jurabib: citations can temporarily switch between bibliography modes;
    // these selectors contain comma-separated citation keys, not prose.
    "nextciteshort" | "nextcitefull" | "nextcitenotitle" | "nextcitereset"
        | "citeswithoutentry" => ("m", 0b1, None),
    "citetitlefortype" | "citenotitlefortype" => ("m", 0b1, None),
    // jurabib's field lookup reads an optional page range, field name and key.
    "citefield" => ("omm", 0b111, None),
    // bibleref.sty uses a small reference language across its book/chapter/
    // verse arguments. Keep each operand intact for that package parser.
    "bibleverse" | "biblechvs" | "biblevs" | "bible" => ("omm", 0b111, None),
    "biblerefcategory" => ("om", 0b11, None),
    "biblerefmap" => ("mm", 0b11, None),
    // bibleref-parse adds a passage parser with a per-call option form.
    "biblerefparseset" => ("m", 0b1, None),
    "pbibleverse" | "pibibleverse" | "pbiblechvs" | "pbiblevs" => ("om", 0b11, None),
    // Harvard's author-as-subject and possessive forms preserve the optional
    // locator and citation key. The second \citeaffixed argument is prose.
    "citeasnoun" | "possessivecite" => ("om", 0b10, None),
    "citeaffixed" => ("omm", 0b010, None),
    "citationmode" | "citationstyle" | "harvardparenthesis" | "harvardyearparenthesis" => ("m", 0b1, None),
    // apacite's classic interface supports an angle pre-note and square
    // post-note before its literal citation-key list.
    "citeA" | "citeNP" | "shortciteA" | "shortciteNP" => ("d<> o m", 0b100, SimpleStar),
    "citeauthorNP" | "citeyearNP" | "shortciteauthor" | "shortciteauthorNP" => ("m", 0b1, SimpleStar),
    // achemso, footbib and mciteplus bibliography controls.
    "bibnote" | "bibnotemark" | "bibnotetext" => ("o", 0b1, None),
    "printbibnotes" | "noibid" => ("", 0, None),
    "footnocite" | "footbibliographystyle" | "footbibliography" => ("m", 0b1, None),
    "mciteSetMidEndSepPunct" | "mciteSetBstMidEndSepPunct" => ("mmm", 0b111, None),
    // Refstyle, titleref and typedref references.
    "newref" | "RSaddto" => ("mm", 0b11, None),
    "titleref" => ("m", 0b1, None),
    "currenttitle" => ("", 0, None),
    "figureref" | "Figureref" | "sectionref" | "Sectionref" | "tableref" | "Tableref"
        | "chapterref" | "Chapterref" | "theoremref" | "Theoremref" | "equationref"
        | "Equationref" => ("m", 0b1, None),
    // Inline bibliography citation forms and the pageranges helper.
    "fullycite" => ("om", 0b10, None),
    "fullcitenext" | "shortcitenext" => ("m", 0b1, None),
    "bibliographycomment" => ("m", 0, None),
    "pages" => ("m", 0b1, None),
    // refcount: counter/reference pairs and expandable reference lookups.
    "setcounterref" | "addtocounterref" | "setcounterpageref" | "addtocounterpageref" => ("mm", 0b11, None),
    "getrefnumber" | "getpagerefnumber" | "setrefcountdefault" => ("m", 0b1, None),
    "getrefbykeydefault" => ("mmm", 0b111, None),
    "IfRefUndefinedExpandable" | "IfRefUndefinedBabel" => ("mmm", 0b001, None),
    // aliascnt and totcount counter declarations and queries.
    "newaliascnt" => ("mm", 0b11, None),
    "aliascntresetthe" => ("m", 0b1, None),
    "newtotcounter" | "regtotcounter" => ("om", 0b11, None),
    "usetotcountfile" => ("m", 0b1, None),
    "addtotcounter" => ("mm", 0b11, None),
    "total" | "totvalue" => ("m", 0b1, None),
    // totalcount defines accessors for counters declared by the user.
    "DeclareTotalCounter" => ("m", 0b1, None),
    "totalfigures" | "totaltables" | "iftotalfigures" | "iftotaltables" => ("", 0, None),
    // backref formatting hooks receive lists and display templates.
    "backref" => ("m", 0, None),
    "backrefalt" => ("mmmm", 0, None),
    "backrefentrycount" => ("mm", 0, None),
    "backrefsetup" => ("m", 0b1, None),
    // crossreftools label metadata and the optional-name/label cross-reference.
    "crtprovidecurrentlabel" | "crtprovidecurrentlabelname" => ("m", 0, None),
    "crtprovidecurrentlabelinfo" => ("mm", 0, None),
    "crtcrossreflabel" => ("omo", 0b100, SimpleStar),
    "crtnameref" => ("m", 0b1, None),
    // perpage counter registration and label listings.
    "MakePerPage" => ("om", 0b11, None),
    "MakeSorted" | "MakeSortedPerPage" => ("m", 0b1, None),
    "listoflabels" => ("o", 0b1, None),
    "showlabel" => ("om", 0b11, None),
    "showrefnames" | "norefnames" => ("", 0, None),
    // theoremref and smartref use one label/counter identifier per lookup.
    "thlabel" | "thref" | "thnameref" => ("m", 0b1, None),
    "addtoreflist" => ("m", 0b1, None),
    "smartref" | "byname" | "byshortname" => ("m", 0b1, None),
    "schapterref" | "srefchapterref" | "partref" | "spartref" | "srefpartref" => ("m", 0b1, None),
    // upref renders a label reference in upright text.
    "upref" => ("m", 0b1, None),
    // amsrefs entries, database selection and reusable field aliases.
    "bib" => ("mmm", 0b111, None),
    "bibselect" => ("m", 0b1, None),
    "DefineName" => ("mm", 0b11, None),
    "DefineJournal" => ("mmmm", 0b1111, None),
    "DefinePublisher" => ("mmmm", 0b1111, None),
    // opcit references and citation state controls.
    "bibref" => ("m", 0b1, None),
    "cited" => ("m", 0b1, None),
    "resetcites" | "hereafter" => ("", 0, None),
    // BibArts full and shortened reference forms.
    "vli" => ("ommmo", 0b00001, None),
    "kli" => ("ommo", 0b0001, None),
    "printvli" | "printvkc" => ("", 0, None),
    // citeref controls the bibliography page list and introduction.
    "bibitempages" => ("m", 0b1, None),
    "bibintro" => ("m", 0, None),
    // Chicago author-date commands take a locator and a key.
    "citeN" | "shortciteN" => ("om", 0b10, None),
    // multibbl bibliography declarations and chbibref heading text.
    "newbibliography" => ("m", 0b1, None),
    "setbibref" => ("m", 0, None),
    // sublabel switches and styles name a counter.
    "sublabon" | "sublaboff" | "substyle" => ("m", 0b1, None),
    // bibtopicprefix sets a reference-label prefix inside btSect.
    "bibprefix" => ("m", 0b1, None),
}

pub(crate) const LIST_ENVIRONMENTS: &[&str] = &["acronym"];
pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &[];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "acronym" | "bibunit" => "o",
        // bibtopic.sty: \begin{btSect}[<style>]{<bib-files>}.
        "btSect" | "category" => "om",
        // amsrefs keeps bibliography styling in environment headers.
        "bibdiv" => "",
        "bibchapter" | "bibsection" => "o",
        "biblist" => "o s g",
        // footnoterange environments do not take a begin header.
        "footnoterange" => "",
        _ => return None,
    })
}

pub(crate) fn environment_for_command(name: &str) -> Option<&str> {
    let environment = name.strip_prefix("end").unwrap_or(name);
    header_signature(environment).map(|_| environment)
}

pub(crate) fn is_package_item(name: &str, environment: &str) -> bool {
    environment == "acronym" && matches!(name, "acro" | "acroplural" | "acroindefinite")
}

fn modifier_at(source: &str, name: &str, cursor: usize, modifier: Modifier) -> bool {
    !name.ends_with('*')
        && matches!(
            (
                modifier,
                source.as_bytes().get(trivia::skip(source, cursor))
            ),
            (
                Modifier::Star | Modifier::Hyper | Modifier::SimpleStar,
                Some(b'*')
            ) | (Modifier::Hyper, Some(b'+'))
        )
}

fn command(source: &str, name: &str, cursor: usize) -> Option<Command> {
    let mut command = fixed_command(name)?;
    if name.trim_end_matches('*') == "newglossary"
        && (name.ends_with('*') || modifier_at(source, name, cursor, command.modifier))
    {
        command.shape = "mm";
        command.data = 0b1;
    }
    Some(command)
}

/// A separated modifier is one token in the compact grammar. Attached stars
/// already belong to the lexer command token and require no extra operand.
pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    let command = command(source, name, cursor)?;
    signature(source, name, cursor, command)
}

fn signature(source: &str, name: &str, cursor: usize, command: Command) -> Option<&'static str> {
    if !modifier_at(source, name, cursor, command.modifier) {
        return Some(command.shape);
    }
    Some(match command.shape {
        "d<> o m" if command.modifier == Modifier::SimpleStar => "s d<> o m",
        "omo" if command.modifier == Modifier::SimpleStar => "somo",
        "m" if command.modifier == Modifier::SimpleStar => "s m",
        "m" => "mm",
        "mm" => "mmm",
        "om" => "mom",
        "oom" => "moom",
        "omm" => "momm",
        "omo" => "momo",
        "mmm" => "mmmm",
        _ => return None,
    })
}

fn contextual_command(name: &str, context: &super::InvocationContext<'_>) -> Option<Command> {
    if let Some((shape, data)) = match name {
        "cite" => Some(("omm", 0b110)),
        "bibliography" => Some(("mmm", 0b011)),
        "bibliographystyle" => Some(("mm", 0b11)),
        _ => None,
    } && context.has_package("multibbl")
    {
        return Some(Command {
            shape,
            data,
            modifier: Modifier::None,
        });
    }
    if matches!(name, "CodeLinkAnchor" | "CodeLinkRef") && context.has_package("xlistings") {
        return Some(Command {
            shape: "om",
            data: 3,
            modifier: Modifier::None,
        });
    }
    // ltxdockit.sty uses a single cross-reference label in changelog entries.
    if name == "see" && context.has_package("ltxdockit") {
        return Some(Command {
            shape: "m",
            data: 1,
            modifier: Modifier::None,
        });
    }
    if !matches!(
        name.trim_end_matches('*'),
        "acs" | "acsp" | "acl" | "Acl" | "aclp" | "Aclp"
    ) {
        return None;
    }
    if context_ambiguous(name, context).is_some() {
        return Some(Command {
            shape: "",
            data: 0,
            modifier: Modifier::None,
        });
    }
    let acro = context.has_package("acro");
    let acronym = context.has_package("acronym");
    if acro && !context.possibly_has_package("acronym") {
        Some(Command {
            shape: "om",
            data: 0b11,
            modifier: Modifier::Star,
        })
    } else if acronym && !context.possibly_has_package("acro") {
        fixed_command(name)
    } else {
        None
    }
}

pub(crate) fn context_ambiguous(
    name: &str,
    context: &super::InvocationContext<'_>,
) -> Option<&'static str> {
    if matches!(name, "cite" | "bibliography" | "bibliographystyle")
        && context.possibly_has_package("multibbl")
        && !context.has_package("multibbl")
    {
        return Some(
            "multibbl changes citation and bibliography argument signatures, or its loading is conditional",
        );
    }
    if matches!(
        name.trim_end_matches('*'),
        "acs" | "acsp" | "acl" | "Acl" | "aclp" | "Aclp"
    ) && (context.possibly_has_package("acro") || context.possibly_has_package("acronym"))
        && !(context.has_package("acro") && !context.possibly_has_package("acronym")
            || context.has_package("acronym") && !context.possibly_has_package("acro"))
    {
        Some(
            "acro and acronym read different optional arguments for this command, or their package loading is conditional",
        )
    } else {
        None
    }
}

pub(crate) fn context_command_signature(
    source: &str,
    name: &str,
    cursor: usize,
    context: &super::InvocationContext<'_>,
) -> Option<&'static str> {
    signature(source, name, cursor, contextual_command(name, context)?)
}

pub(crate) fn context_command_data_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    context: &super::InvocationContext<'_>,
) -> Option<Vec<Range<usize>>> {
    let command = contextual_command(name, context)?;
    Some(domain_ranges(source, name, cursor, command, command.data).unwrap_or_default())
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    fixed_command(name).is_some()
}

fn domain_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    command: Command,
    data: u16,
) -> Option<Vec<Range<usize>>> {
    let mut cursor = cursor;
    if modifier_at(source, name, cursor, command.modifier) {
        cursor = trivia::skip(source, cursor) + 1;
    }
    let mut domains = [b'T'; 16];
    for (index, domain) in domains[..command.shape.len()].iter_mut().enumerate() {
        if data & (1 << index) != 0 {
            *domain = b'U';
        }
    }
    crate::syntax::arguments::domain_ranges(
        source,
        cursor,
        command.shape,
        std::str::from_utf8(&domains[..command.shape.len()]).expect("ASCII operand domains"),
    )
}

/// Protect label/configuration slots individually, including labels following
/// printed optional text. Missing or malformed operands receive no guessed span.
pub(crate) fn command_data_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<Range<usize>>> {
    let command = command(source, name, cursor)?;
    Some(domain_ranges(source, name, cursor, command, command.data).unwrap_or_default())
}

/// These commands execute one supplied continuation. Definitions in either
/// continuation have an uncertain lifetime until the condition is evaluated.
pub(crate) fn command_branch_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<Range<usize>>> {
    let branches = match name {
        "vpagerefcompare" => 0b1100,
        "vpagerefnearby" | "ifglsentryexists" | "ifglsused" | "acroifusedTF" => 0b110,
        "acroifusedT" | "acroifusedF" => 0b10,
        _ => return None,
    };
    let command = command(source, name, cursor)?;
    let slots = crate::syntax::xparse::Signature::parse(command.shape)?
        .slot_ranges(source, cursor..source.len());
    Some(
        slots
            .unwrap_or_default()
            .into_iter()
            .filter_map(|slot| (branches & (1 << slot.slot) != 0).then_some(slot.body))
            .collect(),
    )
}

/// Compatibility for consumers that can represent only a literal prefix.
/// The individual-range callback also covers data after printed arguments.
pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let command = command(source, name, cursor)?;
    let prefix = command
        .shape
        .bytes()
        .enumerate()
        .take_while(|(index, _)| command.data & (1 << index) != 0)
        .count();
    let mut start = cursor;
    if modifier_at(source, name, cursor, command.modifier) {
        start = trivia::skip(source, cursor) + 1;
    }
    let end = crate::syntax::arguments::signature_end(source, start, &command.shape[..prefix])?;
    (end > cursor).then_some(end)
}

/// xv6 book source references are expanded before TeX reads the document.
/// Regex braces, percent signs and math shifts remain literal pattern data.
pub(crate) fn preprocessed_reference_end(source: &str, name: &str, end: usize) -> Option<usize> {
    if !matches!(name, "lineref" | "linerefs") {
        return None;
    }
    let start = crate::trivia::skip(source, end);
    let body = source.get(start..)?.strip_prefix('{')?;
    let colon = body.find(":/")?;
    if body[..colon].contains(['{', '}', '\n', '%']) {
        return None;
    }
    let mut cursor = start + 1 + colon + 2;
    let patterns = if name == "linerefs" { 2 } else { 1 };
    for index in 0..patterns {
        loop {
            match *source.as_bytes().get(cursor)? {
                b'\n' | b'\r' => return None,
                b'\\' if source.as_bytes().get(cursor + 1) == Some(&b'/') => cursor += 2,
                b'/' => {
                    cursor += 1;
                    break;
                }
                _ => cursor += 1,
            }
        }
        if matches!(source.as_bytes().get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
            while source
                .as_bytes()
                .get(cursor)
                .is_some_and(u8::is_ascii_digit)
            {
                cursor += 1;
            }
        }
        if index + 1 < patterns {
            if source.get(cursor..cursor + 2)? != ",/" {
                return None;
            }
            cursor += 2;
        }
    }
    (source.as_bytes().get(cursor) == Some(&b'}')).then_some(cursor + 1)
}
