# Changelog

All notable changes to this project will be documented in this file.

## [2.0.0] — 2026-06-02

A Python **performance toolkit**: static lints plus a sampling runtime profiler.

### Added — profiler power-ups & report UX

- **Exact per-line tracing** — `@line_profile` decorator times every line of a
  function via `sys.settrace` (callee time rolls up to the call site), for the
  microsecond lines the sampler can't catch. `line_profile_result` /
  `line_profile_html`. Opt-in (high overhead, one function at a time).
- **On-demand dump** — `perf_snapshot()` (Rust) reads the current samples without
  stopping; `install_dump_handler()` writes an HTML report on `SIGUSR1`, so a
  long-running server can be dumped live (`kill -USR1 <pid>`).
- **Per-endpoint flamegraphs** — a flamegraph per route, built from the samples
  taken while that endpoint was serving (correlates request spans with stacks).
- **Slow-request capture** — the WSGI/ASGI middleware can auto-write a report for
  any request slower than `slow_request_ms` (snapshot, keeps sampling).
- **SQL query origin** — each recorded query now carries the `file:line` of the
  application code that issued it (shown in the Database section); makes N+1
  sources obvious. `Query.origin`.
- **Multi-worker attach** — `sample_remote_multi([pids…])` / `perf attach --also-pid`
  samples several forked workers at once and merges them (threads kept distinct
  per worker, RSS summed).
- **Faster remote attach** — the interpreter resolution (parse /proc/maps, ELF
  symbols, `_Py_DebugOffsets`) is now cached per pid instead of redone every
  snapshot, so remote sampling collects far more samples/sec. `attach_forget`.
- **Differential flamegraph** in `perf diff` — the after-profile colored by change
  vs before (red = slower, green = faster); folded stacks are now persisted in the
  profile JSON to enable it.
- **Report UX** — function search/filter box, dark-mode toggle, and a one-click
  **functions.csv** download in the HTML report; `export_functions_csv` / CLI
  `perf run --csv`.

### Fixed — release hardening

- **WSGI streaming/timing** — the middleware recorded the request span the instant
  the app returned its (lazy) body iterable, so streaming/generator responses were
  timed at ~0 ms and their status came back as `0`. The body is now wrapped so the
  span is recorded when the server finishes consuming it (or closes it early), and
  the underlying `close()` is forwarded for response cleanup.
- **Sampler memory leak** — `perf_start` used to `Box::leak` a full sample buffer
  on every start, leaking it permanently; repeated `with Profiler()` blocks, the
  signal-dump pattern, and fork profiling all grew memory without bound. Buffers
  are now `Arc`-owned and freed when the run ends.
- **`perf diff` robustness** — missing, malformed, or non-object profile JSON now
  produces a clean error and exit code `1` instead of a traceback;
  `load_profile_json` tolerates unknown `FunctionStat` fields (version skew).
- **`Profiler(trace_memory=True)`** no longer leaves `tracemalloc` running if the
  sampler refuses to start, and never stops `tracemalloc` it did not itself start.

Deferred (roadmap): macOS/Windows attach (needs `mach_vm_read` / Windows APIs —
not validated yet) and a GIL-contention metric (needs interpreter-level
instrumentation to be meaningful).

### Added — runtime profiler (F1 spike)

- **Sampling profiler core (Rust)**: a background thread snapshots every Python
  thread's stack via `sys._current_frames()` (robust across CPython 3.10–3.14),
  plus RSS sampling (Linux). Exposed as `_core.perf_start` / `perf_stop` /
  `perf_running`.
- **`rabbitinspect.perf` module**: `Profiler` context manager, aggregation into
  per-function self/total time, and a self-contained **HTML report** (summary,
  top functions, memory-over-time chart, inline flamegraph, folded stacks).
- **CLI**: `rabbitinspect perf run <script> [--out report.html] [--interval MS]`.

### Added — web framework integration (F2)

- **WSGI / ASGI middleware** (`rabbitinspect.perf_web`): records one request span
  per request (method, route, status, duration) into the running sampler.
  Pass-through no-op when the profiler is off, so it's safe to leave installed.
  Django (`WSGIProfilerMiddleware`), FastAPI/Starlette (`ASGIProfilerMiddleware`).
- **Per-endpoint aggregation**: count, p50/p95/p99, max, and 5xx error count.
- **Request timeline (Gantt)** + endpoint table in the HTML report, color-coded
  by status class.
- **Multi-worker support**: `enable_fork_profiling()` restarts a fresh sampler in
  forked workers (gunicorn/uvicorn) via `os.register_at_fork`, since OS threads
  do not survive `fork()`. New `_core.perf_now_ms` / `perf_record_span` /
  `perf_reset` primitives back this.

### Added — database + hotspot intelligence (F3)

- **Query recording** (`rabbitinspect.perf_db`): `record_query`, `query_timer`
  context manager, and `instrument_sqlalchemy(engine)` / `instrument_django()`
  helpers. Backed by the `_core.perf_record_query` primitive.
- **N+1 detection**: queries are normalized (literals → `?`, `IN (?, ?, …)` →
  `IN (?)`) and grouped per request; repeated shapes within one request are
  flagged per endpoint with max-per-request count and total time. Report gains a
  **Database** section (N+1 offenders + slowest queries).
- **Hotspot ↔ lint cross-reference** (the toolkit's differentiator): the hottest
  functions are run through rabbitinspect's own static analyzer, and findings
  located inside a hot function are surfaced in a **"Hotspots with lint findings"**
  report section — pointing the perf rules straight at the code that dominates
  runtime.

### Added — export, demo & docs

- **Flame chart (time order)** + **call timings**: alongside the aggregate
  flamegraph, the report now reconstructs time-ordered call segments from the
  samples — x is wall-clock time, threads in their own bands, and a function
  called twice shows up as two boxes (not summed). Hover a box for that call's
  duration; click to zoom a time range. A companion "Call timings" table lists,
  per function, the number of calls and total / average / max ms. Remote
  `attach_sample` now reports each thread's `tid` so segments can be split per
  thread. `perf.Segment` / `_build_segments`.
- **Interactive flamegraph**: the HTML report embeds a self-contained icicle
  flamegraph (inline SVG) built from the folded stacks — frame width is the share
  of samples, hover shows sample count / % / ms, **click a frame to zoom into its
  sub-tree** (click the background to reset). The zoom logic is a tiny embedded
  script with no external dependencies. Works for both in-process and
  remote-attach profiles.
- **Per-function memory** (`Profiler(trace_memory=True)` / `perf run --memory`):
  records live allocations via `tracemalloc` at stop and attributes each to its
  function (AST line ranges), shown as a "Top allocations by size" table.
  `ProfileResult.mem_allocations` / `MemAlloc`.
- **Fork-safe `enable_fork_profiling`**: now stops the sampler *before* each fork
  (so the fork happens single-threaded) and restarts it in both parent and child,
  instead of only restarting in the child. Removes the "fork() in a multi-threaded
  process may deadlock" hazard — a sampler thread holding a lock at the fork
  instant could otherwise wedge the child — and the matching DeprecationWarning.
- **Per-line breakdown inside a function**: click any function row in the report
  to expand a line-by-line self-time table (line number, ms, bar, and the actual
  source text), built by attributing each leaf sample to its source line. For a
  view that does `time.sleep(5)`, the sleep line shows ~5000 ms; for CPU code it
  pinpoints the hot statement. `FunctionStat.line_times`.
- **Source line per function**: function tables now show `file:line`, where the
  line is the source line most often sampled for that function — for a hot leaf
  that points straight at the slow statement (e.g. the `time.sleep` line).
  `FunctionStat.line`. Hotspot/lint and tracemalloc sections already had lines.
- **Hotspot section respects `--app-root`**: when an app root is given, the
  "Hotspots with lint findings" section is now scoped to your code (stdlib and
  `site-packages`/`.venv` no longer appear there), matching the function filter.
- **Report table headers** for numeric columns are right-aligned to match their
  values (were left-aligned, looking shifted).
- **Fixed time dilution with multiple threads**: `aggregate` normalized
  per-function time by `len(stacks)` (one stack *per thread per tick*), so a
  function's reported ms was divided by the number of concurrently-sampled
  threads — e.g. a 5 s request on a server with 3 live threads read as ~1.7 s.
  Time is now weighted per *tick* (distinct sample timestamp), so a function that
  ran the whole window reads as the full duration regardless of other threads.
  Single-threaded profiles are unchanged. `self_pct`/`total_pct` are now wall-time
  fractions. (Per-thread time can sum above 100% across threads, as expected.)
- **Application-code filter** (`perf run --app-only` / `perf attach --app-root PATH`):
  a wall-clock sampler over a mostly-idle server is dominated by framework /
  stdlib / idle-thread frames (`socket.readinto`, `selectors.select`, the Django
  autoreloader), burying your own code. The report now adds a "Top application
  functions" section listing only code under your project root (stdlib and
  `site-packages`/`.venv` excluded). `render_html(..., app_root=...)`.
- **asyncio task awareness**: the CPU sampler only sees running OS threads, so
  `await`-ing coroutines (the common case under load) were invisible. New
  `perf.AsyncSampler` / `profile_asyncio(main)` sample the event loop's tasks via
  `Task.get_stack()`, capturing where suspended coroutines are parked; the result
  reuses the normal aggregation, so its flamegraph shows the await hotspots.
  Also `async_task_snapshot()` / `AsyncTaskInfo` for one-shot task census.
- **Speedscope export**: `rabbitinspect perf run … --speedscope profile.json`
  writes a [speedscope](https://speedscope.app) file for an interactive
  flamegraph / time-order view (`perf.to_speedscope`).
- **Profile diff**: `rabbitinspect perf run … --json profile.json` persists a
  profile, and `rabbitinspect perf diff before.json after.json --out diff.html`
  renders a per-function before/after comparison (Δ ms / Δ %, regressions in red,
  improvements in green). API: `perf.diff_profiles` / `render_diff_html` /
  `save_profile_json` / `load_profile_json`.
- **Demo** `examples/perf_demo.py` and a README "Runtime profiler" section.

### Added — attach foundation (F4, step 1)

- **Cross-process introspection** (`rabbitinspect.attach`, Linux): read another
  process's memory via `process_vm_readv`, parse `/proc/<pid>/maps`, and detect
  whether a pid is a CPython process and where its interpreter image is mapped.
  Exposed as `_core.attach_read_mem` / `attach_maps` / `attach_python_info`.
- **Interpreter introspection** (F4 step 2): hand-rolled ELF64 `.dynsym` parsing
  resolves `_PyRuntime` and `Py_Version` in the target, applies the module's
  load bias (ASLR), and reads the exact CPython version out of the live process
  — verified against same-process and cross-process targets. `attach_interpreter_info`.
- **CLI**: `rabbitinspect perf attach --pid <PID>` connects to a running process
  and reports interpreter path, image base, mapping count, CPython version, and
  the `_PyRuntime` address.
- Crate now also builds as `rlib` so `cargo test` runs the reader's unit tests.

- **Remote stack sampling** (F4 step 3): walks the target's threads and
  `_PyInterpreterFrame` chains using offsets read from its own `_Py_DebugOffsets`
  block (so it adapts to the target build rather than hardcoding type layouts),
  recovering function/file/**line** per frame. Line numbers are decoded from each
  code object's `co_linetable` (PEP 626 location-table format) out-of-process.
  `attach_sample` + `perf.sample_remote`.
- **Off-CPU classification** (remote attach): each sampled thread is tagged with
  its OS scheduling state (`/proc/<pid>/task/<tid>/stat`), so the report splits
  self time into on-CPU vs waiting (sleep / blocking I/O / lock). Surfaces as an
  "On-CPU %" card, a per-function "Wait ms" column, and `ProfileResult.on_cpu_ms`
  / `off_cpu_ms` / `FunctionStat.off_cpu_ms`. `attach_sample` now returns
  `{'state', 'frames'}` per thread.
- **Remote memory timeline**: `perf.sample_remote` now samples the target's
  resident memory (`/proc/<pid>/status`) alongside stacks, so the memory-over-time
  chart works for attached processes too — not just in-process runs.
- **Resilient remote sampling**: a frame or code object in the live target can be
  freed/moved between reads, which previously made one transient `EFAULT` abort
  the whole attach session early. The stack walk is now best-effort (a failed
  read ends that one chain, never the snapshot), and `sample_remote` skips a bad
  sample and retries — only stopping when `/proc/<pid>` is gone or after many
  consecutive failures. Attach now runs the full requested duration.
- **CLI**: `rabbitinspect perf attach --pid <PID> --duration <S> --out report.html`
  samples an already-running server with **no code changes** and writes the same
  HTML report (top functions, memory timeline, on/off-CPU, flamegraph,
  hotspot↔lint cross-reference). Verified end-to-end on CPython 3.14 by
  recovering a known call stack.

- **Cross-version attach**: the `_Py_DebugOffsets` field positions are now
  selected per interpreter version (3.13 and 3.14 sub-struct layouts differ).
  Sampling is **validated on CPython 3.13 and 3.14**; 3.11/3.12 predate the
  remote-debug offsets and fail with a clear message (version detection still
  works). Covered by `test_attach_across_python_versions`, which exercises every
  CPython found on PATH.

Notes: timings are statistical (sampling), not exact per-call. Remote attach is
Linux-only and supports CPython 3.13+ (validated on 3.13 and 3.14); per-function
memory attribution remains on the roadmap.

## [1.1.0] - 2026-06-01

### Added

#### New checks (23 new, total 126)

| Code | Rule | Fix |
|------|------|-----|
| RAB077 | `assert` on a tuple literal (always true) | ✅ |
| RAB081 | `return` / `break` / `continue` inside `finally` | ❌ |
| RAB082 | `except BaseException` too broad (catches `SystemExit`/`KeyboardInterrupt`) | ✅ |
| RAB084 | Bare `raise` outside an `except` block | ❌ |
| RAB086 | `x == a or x == b` → `x in (a, b)` | ✅ |
| RAB113 | `eval()` / `exec()` detected (security risk) | ❌ |
| RAB114 | `pickle.load()` / `pickle.loads()` on untrusted data | ❌ |
| RAB115 | `yaml.load()` without `Loader=` (security risk) | ❌ |
| RAB118 | `del` on exception variable (Python 3.12+ clears chain) | ❌ |
| RAB119 | `__init__` in subclass missing `super().__init__()` | ❌ |
| RAB120 | Modifying iterable during iteration | ❌ |
| RAB123 | Deprecated `asyncio.get_event_loop()` / `ensure_future()` | ❌ |
| RAB124 | Blocking call inside async function | ❌ |
| RAB126 | Magic number literal, assign to named constant | ❌ |
| RAB127 | Redundant `else` in loop with `break` | ❌ |
| RAB128 | `not ... in` → `not in` (PEP 8) | ✅ |
| RAB129 | f-string in logging call → lazy `%s` formatting | ✅ |
| RAB130 | `len([... for ...])` → `sum(1 for ...)` (avoid throwaway list) | ✅ |
| RAB131 | `set([...])` / `tuple([...])` / `sorted([...])` / `dict([...])` → generator | ✅ |
| RAB132 | `x = x + [..]` inside a loop (O(n²)) → `.append()` / `.extend()` | ✅ |
| RAB133 | `list.pop(0)` / `list.insert(0, …)` → `collections.deque` | ❌ |
| RAB134 | `sorted(x)[:k]` / `sorted(x)[-k:]` → `heapq.nsmallest` / `nlargest` | ❌ |
| RAB135 | `for … in list(range(…))` → iterate `range(…)` directly | ✅ |

### Changed

- **RAB073**: SQL / gettext templates using only `%s` placeholders are no longer flagged as old-style string formatting
- **RAB092**: Django-style inner config classes (`Meta`, `Options`, `Config`, …) are no longer flagged for missing attribute type annotations
- **Category filtering**: added `--only-category` / `--ignore-category` CLI options to enable or skip groups of related checks

### Performance

- Checkers now declare whether they inspect statements, expressions, or both; the AST walk only dispatches relevant checkers per node instead of all of them
- **RAB096**: unused-import lookup uses a hash set instead of a linear scan per import
- Fewer per-literal string allocations in the magic-number and duplicate-key checks

### Fixed

- **RAB001**: Variables used inside nested scopes (functions/classes) no longer falsely reported as unused
- **RAB029/003/053/070**: Added parentheses when negating complex expressions (`not (a and b)` instead of `not a and b`)
- **RAB067**: Function-name span for built-in shadowing is now located precisely instead of via a fragile substring fallback
- **RAB092**: Assignments inside methods are no longer falsely flagged as class attributes
- **RAB096**: Imports used inside nested scopes no longer falsely reported as unused
- **RAB105**: Added scope stack to prevent state leaking from nested functions
- **RAB109**: Classes with leading underscore (`_MyClass`) correctly recognized as CamelCase
- **Autofix**: removed a dead/duplicated branch in overlapping-fix handling
- **CI**: Removed broken `build-wheel` job; each test job now builds its own compatible wheel

## [1.0.0] - 2026-05-31

### Added

#### New checks (18 new, total 107)

| Code | Rule | Fix |
|------|------|-----|
| RAB090 | Public function parameter missing type annotation | ❌ |
| RAB091 | Function missing return type annotation | ❌ |
| RAB092 | Class attribute missing type annotation | ❌ |
| RAB093 | Module-level variable missing type annotation | ❌ |
| RAB094 | `Any` type annotation used, prefer concrete type | ❌ |
| RAB095 | Default value incompatible with type annotation | ❌ |
| RAB096 | Unused import | ❌ |
| RAB097 | Debugging `print()` / `breakpoint()` / `pdb.set_trace()` left in code | ❌ |
| RAB098 | Import inside function/class body, move to module level | ❌ |
| RAB099 | Duplicate key/element in dict/set literal | ❌ |
| RAB100 | Redundant `elif` after `return`/`raise`/`break`/`continue` | ❌ |
| RAB103 | Self-comparison (`x == x`) always True/False | ❌ |
| RAB104 | Pass-through generator `list(x for x in y)` → `list(y)` | ❌ |
| RAB105 | Inconsistent return statements (mixed bare and valued) | ❌ |
| RAB106 | Too broad `except Exception:` catch | ❌ |
| RAB107 | TODO/FIXME/HACK/XXX comment left in code | ❌ |
| RAB108 | `__all__` contains non-string elements | ❌ |
| RAB109 | Class name should use CamelCase convention | ❌ |
| RAB110 | Function name should use snake_case convention | ❌ |
| RAB111 | Module-level constant should use UPPER_CASE naming | ❌ |
| RAB112 | Unnecessary `pass` in non-empty body | ✅ |

### Fixed
- `assert_no_findings` now ignores all new rule codes by default to avoid breaking existing tests
- All fixture files updated with bad examples for new rules

## [0.2.0] - 2026-05-31

### Added

#### Infrastructure
- `--select` / `--ignore` CLI flags for filtering checks
- `--format json` output option
- `# noqa: RABNNN` inline comment suppression
- `[tool.rabbitinspect]` configuration via `pyproject.toml`
- `tomli` fallback for Python 3.10 compatibility
- GitHub Actions CI with lint (ruff + ty), Rust build, wheel build, and multi-Python testing (3.10–3.14)
- GitHub Actions publish workflow with cibuildwheel for Linux (x86_64 + aarch64), macOS (x86_64 + arm64), and Windows
- Type stub (`_core.pyi`) for the compiled Rust extension
- Dev dependencies: `ruff`, `ty`, `tomli`

#### Rust refactoring
- `iter_fn_args()` helper eliminates repeated `posonlyargs.iter().chain(args.args.iter()).chain(kwonlyargs.iter())` pattern across 7 checkers
- `count_fn_args()` helper for counting function parameters
- Overlapping fix detection — `apply_fixes` skips fixes that overlap with already-applied regions

#### New checks (46 total)

| Code | Rule | Fix |
|------|------|-----|
| RAB046 | `sorted(x)[0]` → `min(x)` | ✅ |
| RAB047 | `sorted(x)[-1]` → `max(x)` | ✅ |
| RAB048 | `not x is None` → `x is not None` | ✅ |
| RAB049 | `x = x + 1` → `x += 1` | ✅ |
| RAB050 | `for i in range(len(seq))` — iterate directly | ❌ |
| RAB051 | `d.setdefault(k, []).append(v)` → `defaultdict` | ❌ |
| RAB052 | `type(x) == A or type(x) == B` → `isinstance(x, (A, B))` | ✅ |
| RAB053 | `x is True` / `x is False` → `x` / `not x` | ✅ |
| RAB054 | `if not x: x = y` → `x = x or y` | ✅ |
| RAB055 | Unused loop variable → `_` | ✅ |
| RAB056 | Nested `with` statements | ❌ |
| RAB057 | `s.startswith('a') or s.startswith('b')` → `s.startswith(('a', 'b'))` | ✅ |
| RAB058 | `return True if cond else False` → `return cond` | ✅ |
| RAB059 | `while True:` without `break` → infinite loop | ❌ |
| RAB060 | `sorted(x).sort()` → `x.sort()` | ✅ |
| RAB061 | `from module import *` — wildcard import | ❌ |
| RAB062 | Redundant `pass` after docstring | ✅ |
| RAB063 | `x is 5` / `x is "str"` → `x == 5` / `x == "str"` | ✅ |
| RAB064 | `__init__` returning non-None value | ✅ |
| RAB065 | `if True:` / `if False:` dead code | ❌ |
| RAB066 | Function definition inside a loop | ❌ |
| RAB067 | Variable/function shadows built-in name | ✅ |
| RAB068 | `raise Exc()` without `from` inside `except` | ❌ |
| RAB069 | `dict()` / `list()` / `tuple()` → `{}` / `[]` / `()` | ✅ |
| RAB070 | `x == ""` / `x == []` / `x == {}` → `not x` / `x` | ✅ |
| RAB071 | List comp inside `str.join()` → generator | ✅ |
| RAB072 | Except handler only re-raises | ❌ |
| RAB073 | Old-style `%` string formatting | ❌ |
| RAB074 | `os.path.*` → `pathlib.Path` | ❌ |
| RAB075 | `isinstance(x, (A,))` → `isinstance(x, A)` | ✅ |
| RAB076 | `str()` on value already a string | ✅ |
| RAB078 | `except Exception: pass` — silent swallow | ❌ |
| RAB079 | `__del__` method defined | ❌ |
| RAB080 | `list(d.keys())` / `list(d.values())` → `list(d)` | ✅ |
| RAB083 | Nested ternary expression | ❌ |
| RAB085 | `reversed(sorted(x))` → `sorted(x, reverse=True)` | ✅ |
| RAB087 | `{k: v for k, v in zip(...)}` → `dict(zip(...))` | ✅ |
| RAB088 | `while len(x) > 0` → `while x` | ✅ |
| RAB089 | `copy.copy(x)` → `x.copy()` | ✅ |
| RAB097 | Debugging `print()` / `breakpoint()` / `pdb.set_trace()` left in code | ❌ |
| RAB098 | Import inside function/class body, move to module level | ❌ |
| RAB099 | Duplicate key/element in dict/set literal | ❌ |
| RAB106 | Too broad `except Exception:` catch | ❌ |
| RAB112 | Unnecessary `pass` in non-empty body | ✅ |
| RAB090 | Public function parameter missing type annotation | ❌ |
| RAB091 | Function missing return type annotation | ❌ |
| RAB092 | Class attribute missing type annotation | ❌ |
| RAB093 | Module-level variable missing type annotation | ❌ |
| RAB094 | `Any` type annotation used, prefer concrete type | ❌ |
| RAB095 | Default value incompatible with type annotation | ❌ |
| RAB100 | Redundant `elif` after `return`/`raise`/`break`/`continue` | ❌ |
| RAB103 | Self-comparison (`x == x`) always True/False | ❌ |
| RAB104 | Pass-through generator `list(x for x in y)` → `list(y)` | ❌ |
| RAB107 | TODO/FIXME/HACK/XXX comment left in code | ❌ |
| RAB109 | Class name should use CamelCase convention | ❌ |
| RAB110 | Function name should use snake_case convention | ❌ |
| RAB111 | Module-level constant should use UPPER_CASE naming | ❌ |
| RAB096 | Unused import | ❌ |
| RAB105 | Inconsistent return statements (mixed bare and valued) | ❌ |
| RAB108 | `__all__` contains non-string elements | ❌ |

- Initial release with 43 checks (RAB001–RAB045, RAB101–RAB102)
- Rust+PyO3 analysis engine
- CLI with ruff-style output and `--fix` flag
- Auto-fix support for 22 checks
- Comprehensive test suite (255 tests)
- Native generics (RAB039) and union syntax (RAB044) checks
- Cognitive and cyclomatic complexity analysis

### Fixed

#### Bugs
- **RAB001**: `x += 1` no longer incorrectly reports `x` as unused (augmented assignment target is now also recorded as used)
- **RAB040**: `@dataclass(frozen=True)` now correctly produces `@dataclass(frozen=True, slots=True)`, preserving existing keyword arguments
- **RAB060**: `sorted(x, reverse=True).sort()` is now skipped (incorrect semantics)
- **RAB067**: Function definition fix no longer destroys the function body (only the name is replaced)
- **RAB068**: Uses a counter stack instead of a boolean for `enter_except`/`exit_except`, correctly handling nested except handlers; resets on function scope boundaries
- **B5**: `apply_fixes` now handles overlapping fix ranges by skipping fixes that overlap with already-applied regions (e.g., RAB052 + RAB006 on `type(x) == int or type(x) == str`)
- **B7**: `contains_name_ref` and `stmt_contains_name_ref` now handle `Await`, `Yield`, `YieldFrom`, `Match`, `ClassDef`, `AsyncFor`, `Import`, `ImportFrom`, `TypeAlias` — eliminating false positives in RAB055
- **B10**: `tomllib` import has a `try/except` fallback to `tomli` for Python 3.10

#### Code quality
- ComplexityChecker, FunctionLengthChecker, TooManyParamsChecker, CognitiveComplexityChecker now report the actual function position instead of `(0, 0)`
- RAB055 now handles `AsyncFor` loops in addition to `For`
- Fixture files have `# ruff: noqa` headers to avoid false positives in linting
- Duplicate `bad_range_len` function in test fixtures renamed

### Documentation
- Complete README rewrite with all 89 checks, CLI flags, `# noqa`, `pyproject.toml` config, and expanded performance rationale table
- CHANGELOG added
