use super::{BTreeMap, CommandRule};

pub(super) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "thmbox" => "om",
        _ => return None,
    })
}

commands! {
    // environ.sty:19--23. The body name is a token; final code is delayed.
    "environbodyname" | "environfinalcode" => ("m", "m"),
    // thm-kv.sty:333--342 permits options on both sides of the name.
    "declaretheorem" => ("omo", "omo"),
    "declaretheoremstyle" => ("om", "om"),
    "addtotheorempreheadhook" | "addtotheorempostheadhook"
        | "addtotheoremprefoothook" | "addtotheorempostfoothook" => ("om", "om"),
    // ntheorem.sty:435--511,977--1085. Header/foot hooks are delayed.
    "theoremstyle" | "theorembodyfont" | "theoremheaderfont" | "theoremsymbol"
        | "theoremseparator" | "theoremprework" | "theorempostwork"
        | "theoremnumbering"
        | "theorempreskip" | "theorempostskip" | "theoremlisttype"
        | "listtheorems" => ("m", "m"),
    "newtheoremlisttype" | "renewtheoremlisttype" => ("mmmm", "mmmm"),
    // thmbox.sty:15. Theorem factory grammar is handled by the declaration reader.
    "thmboxoptions" => ("m", "m"),

}
