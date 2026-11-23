# Building from source

The checkout selects Rust 1.99.1. To build with your installed stable toolchain:

```sh
cargo +stable build --release --locked
```

The executable is `target/release/tift` on Linux and macOS, or `target/release/tift.exe` on Windows. Install it from a local checkout with:

```sh
cargo +stable install --path . --locked
```

Generate the library API reference with:

```sh
cargo +stable doc --no-deps --locked --open
```

The [installation guide](Getting-Started) covers platform requirements and release downloads. See [library usage](Library-API) for integrating tift into a Rust application.
