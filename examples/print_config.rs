fn main() {
    print!(
        "{}",
        tift::Config::default_toml().expect("serialize built-in configuration")
    );
}
