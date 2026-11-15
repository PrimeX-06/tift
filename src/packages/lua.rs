//! Literal luadraw and luacode grammar from their installed package sources.
//!
//! luadraw.sty 3.7 (2026/11/05) delegates its Lua body to luacode's
//! line reader, with a soft TeX catcode table or the all-other starred table.

use crate::config::CommandRule;
use std::collections::BTreeMap;

pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &["luadraw", "luadraw*", "luacode", "luacode*"];

pub(crate) const LUA_CODE_COMMANDS: &[&str] = &["luaexec", "luadirect"];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // luadraw.sty declares both forms with [2][]. The optional first
        // argument is unused; the required group carries name/exec/auto keys.
        "luadraw" | "luadraw*" => "om",
        "luacode" | "luacode*" => "",
        _ => return None,
    })
}

/// luacode@grab@lines compares its stripped physical line with the end marker.
/// An end spelling inside a Lua string or comment remains ordinary code.
pub(crate) fn is_line_delimited_environment(name: &str) -> bool {
    VERBATIM_ENVIRONMENTS.contains(&name)
}

pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // luadraw.sty: literal PDF drawing instructions passed to pgfsys.
            ("luadrawLiteral", 1, false),
            // luacode.sty: execution aliases and quoted TeX-to-Lua string helpers.
            ("luaexec", 1, false),
            ("luadirect", 1, false),
            ("luastring", 1, false),
            ("luastringO", 1, false),
            ("luastringN", 1, false),
            ("LuaCodeDebugOn", 0, false),
            ("LuaCodeDebugOff", 0, false),
        ],
    );
}
