# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] — 2.0 (in progress)

Toward a Python **performance toolkit**: static lints plus a runtime profiler.

### Added — runtime profiler (F1 spike)

- **Sampling profiler core (Rust)**: a background thread snapshots every Python
  thread's stack via `sys._current_frames()` (robust across CPython 3.10–3.14),
  plus RSS sampling (Linux). Exposed as `_core.perf_start` / `perf_stop` /
  `perf_running`.
- **`rabbitinspect.perf` module**: `Profiler` context manager, aggregation into
  per-function self/total time, and a self-contained **HTML report** (summary,
  top functions, memory-over-time chart, folded stacks for flamegraph tools).
- **CLI**: `rabbitinspect perf run <script> [--out report.html] [--interval MS]`.

Notes: timings are statistical (sampling), not exact per-call. Roadmap: web
framework request spans (Django/FastAPI), multi-worker, SQL/N+1, off-CPU,
hotspot↔lint cross-reference, and attach-to-PID.

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
