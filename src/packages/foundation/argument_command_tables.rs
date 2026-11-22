//! Compact declarations shared by package providers with literal prefixes.

macro_rules! argument_commands {
    ($( $($name:literal)|+ => ($shape:literal, $data:literal) ),+ $(,)?) => {
        pub(super) fn specification(name: &str) -> Option<(&'static str, &'static str)> {
            Some(match name.trim_end_matches('*') {
                $( $($name)|+ => ($shape, $data), )+
                _ => return None,
            })
        }

        pub(super) fn add_command_rules(
            commands: &mut BTreeMap<String, CommandRule>,
        ) {
            $( for name in [$($name),+] {
                commands.insert(name.into(), CommandRule {
                    signature: Some($shape.into()),
                    ..Default::default()
                });
            } )+
        }
    };
}

pub(crate) use argument_commands;
