# Editor and repository integration

Install tift on `PATH` before configuring an editor. Whole-document stdin with the actual document path provides file-kind selection, configuration discovery, overrides, and root-comment context:

```sh
tift - --stdin-filename /path/to/main.tex
```

Formatted text goes to stdout and diagnostics to stderr. The virtual path may refer to an unsaved file.

## Visual Studio Code / LaTeX Workshop

```json
{
    "latex-workshop.formatting.latex": "latexindent",
    "latex-workshop.formatting.latexindent.path": "tift",
    "latex-workshop.formatting.latexindent.args": ["--print", "%TMPFILE%"],
    "[latex]": {
        "editor.defaultFormatter": "James-Yu.latex-workshop",
        "editor.formatOnSave": true
    }
}
```

LaTeX Workshop's latexindent adapter sends a temporary file in the document directory. Configuration discovery uses that directory; filename-specific overrides see the temporary name. For exact path-based overrides, use an editor integration that supplies stdin and the original document path.

Open the command palette and choose **Preferences: Open Workspace Settings (JSON)** to merge these settings. Open a `.tex` file, then run **Format Document** or save it. The default shortcut is **Shift+Alt+F** on Windows/Linux and **Shift+Option+F** on macOS. Set `editor.formatOnSave` to `false` for manual formatting.

The [VS Code guide](VS-Code) covers installation, absolute paths, configuration, and troubleshooting.

## Neovim / Conform

```lua
require("conform").setup({
  formatters_by_ft = {
    tex = { "tift" },
    bib = { "tift" },
  },
  formatters = {
    tift = {
      command = "tift",
      args = { "-", "--stdin-filename", "$FILENAME" },
      stdin = true,
    },
  },
})
```

Add formatting-on-save through your normal Conform setup. Root comments in chapter buffers remain available to tift.

## pre-commit

Add a local hook to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: local
    hooks:
      - id: tift
        name: tift
        entry: tift --check
        language: system
        types: [file]
        files: '\.(tex|tikz|sty|cls|bib|dtx|ins)$'
```

Install the hook with `pre-commit install`. Run `pre-commit run --all-files` to check the repository. This hook requires an installed tift executable. Change `entry` to `tift` if your preferred hook writes formatting before a commit.

## GitHub Actions for a LaTeX project

```yaml
name: LaTeX formatting
on: [push, pull_request]
permissions:
  contents: read
jobs:
  format:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.99.1
      - name: Install tift
        run: cargo install tift --version 0.1.4 --locked --registry crates-io
      - name: Check formatting
        run: tift --no-cache --check .
```

Pin a release version to keep shared project tooling consistent. Commit `tift.toml` and ignore generated TeX outputs. A formatting difference returns 1 and fails the check.

## Selections

Editors with one-based line selections can pass `--lines '10-25'` while sending the whole document. Parsing and width calculation use complete-document context; unselected lines retain their bytes. Cross-boundary changes are retained and reported, so extend a selection to the full paragraph or block when required.
