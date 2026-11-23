# Cache, line selections, and tracing

## Persistent formatting results

The CLI enables persistent result files by default. Results are keyed by exact source bytes, effective settings, file kind, and executable content hash. Current project declaration context is prepared before lookup, so dependency changes affect formatting.

```toml
[cache]
enabled = true
directory = ".tift-cache"
```

A configured directory is relative to the configuration declaring it. `--cache-dir PATH` takes precedence and uses the CLI working directory for relative paths. Add a project-local cache directory to `.gitignore`.

| Platform | Default cache directory |
|---|---|
| Windows | `%LOCALAPPDATA%/tift/cache` |
| macOS | `~/Library/Caches/tift` |
| Linux/Unix | `$XDG_CACHE_HOME/tift` or `~/.cache/tift` |

`TIFT_CACHE_HOME` overrides the default. An empty value disables default-directory selection; explicit configuration and CLI paths still apply.

JSON files under `v2/` group inputs by source directory. Each input retains its latest actual input-to-output result. Groups are read once per invocation and updated when needed. Nonblocking file locks coordinate writers, and records use temporary files and atomic replacement.

Corrupt records, unavailable storage, or lock contention fall back to actual formatting. Failures are excluded from cache storage. Ordinary summaries show hit/miss counts. `--quiet` suppresses summaries and warnings.

## Disabling storage

```sh
tift --no-cache --check .
```

Or set `[cache] enabled = false`. Source analysis, compiled configuration, and other in-memory reuse continue. `--trace`, `--trace-detail`, and `--lines` run actual formatting directly.

A changed in-place file needs one actual pass on its new source before that source's result can be reused. This preserves replacement rules intentionally producing successive changes.

## Line ranges

```sh
tift --lines '3,7-12,!9' main.tex
```

Selections are one-based and accept singles, closed ranges, unions, and exclusions. Full-document context provides structure and alignment widths. Unselected physical lines retain their bytes. When an edit moves content across the boundary, tift keeps the original source for that edit and emits a warning. Select the whole block or paragraph to permit the transformation.

## Timing and detailed edits

```sh
tift --trace --print main.tex
tift --trace-detail --print main.tex
tift --trace-detail --lines '20-35' --print main.tex
```

Trace output uses stderr, keeping formatted stdout usable by editors and pipes. Stage logs include changes, byte counts, and durations. Detailed records include actual before/after text, structure, and associated settings. Coordinates are local to each stage; detailed line ranges are one-based and end-exclusive.

With line selections, detailed records describe applied selection edits and stage timings cover full-document processing. Trace output can contain document content, so inspect excerpts before sharing diagnostics publicly.

## Diagnosis workflow

1. Print effective settings with `tift --show-config main.tex`.
2. Preview source changes with `tift --diff main.tex`.
3. Inspect detailed edits with `tift --trace-detail --print main.tex`.
4. Reduce the case to a small source excerpt and its required declarations.
5. Adjust a name rule or protection entry, then check the formatted result again.

The [configuration reference](Configuration) describes stage settings and the [library API](Library-API) exposes trace records to applications.
