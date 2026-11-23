# Project processing

tift prepares local dependencies and declaration context before formatting project files. Source files, dependency lookups, and effective declaration configurations are shared within each invocation.

## Worker threads

Independent files can use configurable worker threads:

```sh
tift --threads 4 .
tift --threads 1 --check .
```

Project declarations are resolved before file formatting so local command and environment definitions are available to each file.

## Repeated runs

Persistent result caching stores results for exact source bytes, effective settings, file kind, and executable content. Dependency changes participate in the effective project context.

```sh
tift --check .
tift --no-cache --check .
```

The [cache and tracing guide](Cache-and-Tracing) describes storage, invalidation, and diagnostics. The [project context guide](Project-Context) covers roots, local imports, and declaration inference.

## Measuring formatting work

Use `--no-cache` to measure actual formatting rather than a result-cache hit. `--trace` reports project preparation and formatting stage times without changing the result:

```sh
tift --no-cache --trace --check main.tex
tift --no-cache --threads 1 --check chapters/
```

Measure several repeated runs and report a median. Small single-file CLI jobs include process loading, configuration discovery and dependency preparation, so their total latency can be dominated by startup. A library benchmark can reuse `CompiledConfig` and measure `format` calls with file reads outside the timed interval. Record the compiler, release options, CPU, input sizes and whether cache or worker threads were enabled.

English and CJK documents use the same pipeline. Display-column calculation matters for mixed-width text, tables and wrapping; use `alignment.width = "display"` when terminal display width is the desired measure. A benchmark should keep settings and source files identical across versions, include both language groups, and report any retained or failed inputs separately from successful formatting throughput.

Source reconstruction, formatting idempotence and error recovery are separate checks. A file returned unchanged with an error has not completed formatting. Compare repeated outputs, inspect diagnostics and compile representative documents to validate more than timing alone.
