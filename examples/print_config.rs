use tift::Config;

fn main() {
    print!(
        "{}",
        toml::to_string_pretty(&Config::default()).expect("serialize built-in configuration")
    );
}
