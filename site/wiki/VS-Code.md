# VS Code and LaTeX Workshop

Use tift through LaTeX Workshop's configurable `latexindent` adapter. The adapter runs tift on a temporary copy of the editor document and reads formatted text from stdout.

## Install and select the executable

Install tift with Rust 1.99.1 or later:

```sh
cargo install tift --version 0.1.4 --locked --registry crates-io
tift --version
```

Install [LaTeX Workshop](https://marketplace.visualstudio.com/items?itemName=James-Yu.latex-workshop) in VS Code. Open your LaTeX project folder and verify `tift --version` in VS Code's integrated terminal. Restart VS Code after changing `PATH`, so the extension receives the updated environment.

Cargo normally installs tift under `~/.cargo/bin` or `%USERPROFILE%\.cargo\bin`. Use an absolute path if VS Code cannot find it. In PowerShell, `(Get-Command tift).Source` prints the installed executable path. JSON paths accept forward slashes; a generic Windows example is `"C:/Tools/tift/tift.exe"`.

## Workspace settings

Run **Preferences: Open Workspace Settings (JSON)** from the command palette: **Ctrl+Shift+P** on Windows/Linux or **Cmd+Shift+P** on macOS. Merge the following into `.vscode/settings.json`, preserving other project settings:

```json
{
    "latex-workshop.formatting.latex": "latexindent",
    "latex-workshop.formatting.latexindent.path": "tift",
    "latex-workshop.formatting.latexindent.args": [
        "--print",
        "%TMPFILE%"
    ],
    "[latex]": {
        "editor.defaultFormatter": "James-Yu.latex-workshop",
        "editor.formatOnSave": true
    }
}
```

| Setting | Purpose |
|---|---|
| `latex-workshop.formatting.latex` | Select the configurable `latexindent` adapter |
| `latex-workshop.formatting.latexindent.path` | Select the tift executable |
| `latex-workshop.formatting.latexindent.args` | Replace adapter defaults with tift arguments |
| `%TMPFILE%` | Supply the temporary source file created by the extension |
| `--print` | Return formatted text through stdout |
| `[latex].editor.defaultFormatter` | Use LaTeX Workshop for LaTeX buffers |
| `[latex].editor.formatOnSave` | Format `.tex` documents when saving |

Current settings use the `latex-workshop.formatting.latexindent.*` prefix. Older `latex-workshop.latexindent.*` names are deprecated.

## Format a document

1. Open a `.tex` document and confirm the language mode is **LaTeX** in the status bar.
2. Run **Format Document** from the command palette, or press **Shift+Alt+F** on Windows/Linux or **Shift+Option+F** on macOS.
3. If VS Code asks for a provider, choose **LaTeX Workshop**.
4. Save the document. With the example configuration, saving also runs formatting.

For manual formatting, set `editor.formatOnSave` to `false`. Formatting updates the editor buffer; ordinary saving writes it to the source file. tift's `--print` mode retains the adapter's temporary input.

## Project formatting rules

Place `tift.toml` beside the main source:

```toml
[indent]
width = 4

[wrap]
mode = "preserve"
columns = 88

[whitespace]
line-ending = "preserve"
final-newline = "preserve"
```

Use `mode = "wrap"` to wrap overlong prose while keeping existing line boundaries, or `mode = "rewrap"` to join old prose lines and wrap whole paragraphs. The same configuration applies to CLI runs and editor formatting. The adapter's temporary file is in the document directory, so ordinary project discovery follows that directory. Filename-specific `[[overrides]]` see the temporary name.

For chapter declarations, add a root comment:

```tex
% !TEX root = ../main.tex
```

The path is relative to the chapter. Supported local inputs, classes, and packages contribute declaration context. See [Project context](Project-Context).

Command-line arguments can override project rules. For example, use this `args` array for two-space indentation and explicit 88-column wrapping:

```json
[
    "--print",
    "--indent-width", "2",
    "--wrap", "--wrap-width", "88",
    "%TMPFILE%"
]
```

For an explicit shared configuration, use `--config` with an absolute path:

```json
[
    "--print",
    "--config", "C:/Projects/paper/tift.toml",
    "%TMPFILE%"
]
```

## Troubleshooting

| Symptom | Check |
|---|---|
| Executable not found or `ENOENT` | Verify `tift --version` in the VS Code terminal, restart after changing `PATH`, or set an absolute executable path |
| Wrong formatter selected | Set `[latex].editor.defaultFormatter`, then use **Format Document With...** and select LaTeX Workshop |
| Unknown command-line option | Replace the entire adapter `args` array with `--print` and `%TMPFILE%`, then add supported tift flags |
| Formatting unavailable | Confirm the buffer's language mode is LaTeX and LaTeX Workshop is enabled |
| Prose width unchanged | Enable `--wrap` or `wrap.mode = "wrap"`; a width setting alone preserves prose boundaries |
| Unexpected configuration | Run `tift --show-config path/to/main.tex`; inspect nearest `tift.toml`, inheritance, overrides, and CLI flags |
| Chapter custom syntax misread | Check its `% !TEX root` path and local declaration imports |
| Save does not format | Check `[latex].editor.formatOnSave`, the selected formatter, and the LaTeX Workshop output log |
| Docker adapter runs a different executable | Local tift integration uses `latex-workshop.docker.enabled = false`; this setting also controls LaTeX Workshop's Docker build integration |

Open **View: Toggle Output** and select **LaTeX Workshop** to inspect executable and formatting diagnostics. Reproduce source behavior in the terminal with `tift --diff path/to/main.tex` or `tift --trace-detail --print path/to/main.tex`.

TeX engines support LaTeX Workshop's document-building features. Source formatting uses the tift executable. Use [formatting controls](Supported-Syntax#formatting-controls) for source regions that require literal preservation.
