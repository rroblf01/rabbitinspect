# rabbitinspect

A **Python code performance analyzer** written in **Rust** (via PyO3). It scans your Python source code and points out patterns that can be made faster, more memory-efficient, or more idiomatic — with optional automatic fixes.

---

## Installation

```bash
pip install rabbitinspect
```

The package ships pre-compiled wheels — no Rust toolchain required.

---

## Usage

### CLI

```bash
# Check a single file
rabbitinspect file.py

# Check an entire directory
rabbitinspect src/

# Auto-fix detected issues
rabbitinspect file.py --fix

# Select specific checks only
rabbitinspect file.py --select RAB002,RAB003

# Ignore specific checks
rabbitinspect file.py --ignore RAB022,RAB101

# JSON output
rabbitinspect file.py --format json

# Disable colored output
rabbitinspect file.py --no-color
```

Output (ruff-style):

```
file.py:10:5: RAB002 Use 'is' instead of '==' for None comparison
file.py:25:8: RAB003 Use 'not x' instead of 'len(x) == 0' for emptiness check
```

### Available commands

| Command | Description |
|---------|-------------|
| `rabbitinspect .` | Analyze all Python files recursively |
| `rabbitinspect file.py --fix` | Analyze and auto-fix |
| `rabbitinspect --explain RAB002` | Show detailed explanation for a check |
| `rabbitinspect --list` | List all available checks grouped by category |
| `rabbitinspect --select RAB002,RAB003 file.py` | Run only specific checks |
| `rabbitinspect --ignore RAB022,RAB101 file.py` | Skip specific checks |
| `rabbitinspect --only-category correctness file.py` | Run only checks in given categories |
| `rabbitinspect --ignore-category style,complexity file.py` | Skip entire categories |
| `rabbitinspect --format json file.py` | JSON output |
| `rabbitinspect --no-color file.py` | Disable colored output |

### Suppressing checks

Inline with `# noqa`:

```python
x = 42  # noqa: RAB067
try:
    pass
except:
    pass  # noqa: RAB012, RAB013
```

Or globally in `pyproject.toml`:

```toml
[tool.rabbitinspect]
select = ["RAB002", "RAB003", "RAB006"]
ignore = ["RAB022", "RAB101"]
```

```python
x = 42  # noqa: RAB067
try:
    pass
except:
    pass  # noqa: RAB012, RAB013
```

### Configuration via pyproject.toml

```toml
[tool.rabbitinspect]
select = ["RAB002", "RAB003", "RAB006"]
ignore = ["RAB022", "RAB101"]
only-categories = ["correctness", "performance"]
ignore-categories = ["style", "complexity"]
```

### Python API

```python
from rabbitinspect import analyze_code, apply_fixes

source = """
def foo(x):
    if x == None:
        return True
    return False
"""

findings = analyze_code(source)
for f in findings:
    print(f"{f['code']}:{f['line']}:{f['col']} {f['message']}")

# Apply fixes
fixes = [f['fix'] for f in findings if f.get('fix')]
fixed = apply_fixes(source, fixes)
```

---

## Rule Categories

Rules are grouped into categories so you can enable or disable related checks together:

| Category | Description | Count |
|----------|-------------|-------|
| **correctness** | Potential bugs and correctness issues | 51 |
| **performance** | Performance improvements | 35 |
| **style** | Code style and conventions | 25 |
| **typesafety** | Type annotation rules | 7 |
| **complexity** | Code complexity metrics | 4 |
| **import** | Import-related rules | 3 |
| **unused** | Unused variables and imports | 1 |

Use `--only-category` to run only checks in specific categories, or `--ignore-category` to skip entire categories:

```bash
# Only show correctness and performance issues
rabbitinspect src/ --only-category correctness,performance

# Skip style and complexity checks
rabbitinspect src/ --ignore-category style,complexity
```

## Checks

### correctness — Potential bugs and correctness issues

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB002** | `x == None` instead of `x is None` | ✅ |
| **RAB003** | `len(x) == 0` instead of `not x` | ✅ |
| **RAB006** | `type(x) == T` instead of `isinstance(x, T)` | ✅ |
| **RAB007** | Unnecessary `else` after `return`/`raise`/`break`/`continue` | ✅ |
| **RAB011** | Mutable default argument (`x=[]`, `x={}`) | ❌ |
| **RAB012** | Bare `except:` clause | ❌ |
| **RAB013** | Bare `except: pass` | ❌ |
| **RAB025** | Long if-elif chain (> 3 branches) | ❌ |
| **RAB029** | `x == True` / `x == False` instead of `x` / `not x` | ✅ |
| **RAB030** | `if cond: return True else: return False` → `return cond` | ✅ |
| **RAB031** | `if k in d: return d[k]` → `d.get(k)` | ✅ |
| **RAB034** | `assert True` / `assert False` | ✅ |
| **RAB035** | `x[:]` → `x.copy()` for list copies | ✅ |
| **RAB045** | `open()` without context manager (`with`) | ❌ |
| **RAB048** | `not x is None` → `x is not None` | ✅ |
| **RAB052** | `type(x) == A or type(x) == B` → `isinstance(x, (A, B))` | ✅ |
| **RAB054** | `if not x: x = y` → `x = x or y` | ✅ |
| **RAB057** | `s.startswith('a') or s.startswith('b')` → `s.startswith(('a', 'b'))` | ✅ |
| **RAB059** | `while True:` without `break` → infinite loop | ❌ |
| **RAB063** | `x is 5` / `x is "str"` → `x == 5` / `x == "str"` | ✅ |
| **RAB064** | `__init__` returning non-None value | ✅ |
| **RAB065** | `if True:` / `if False:` dead code | ❌ |
| **RAB068** | `raise Exc()` without `from` inside `except` | ❌ |
| **RAB070** | `x == ""` / `x == []` / `x == {}` → `not x` / `x` | ✅ |
| **RAB072** | Except handler only re-raises | ❌ |
| **RAB075** | `isinstance(x, (A,))` → `isinstance(x, A)` | ✅ |
| **RAB076** | `str()` on value already a string | ✅ |
| **RAB077** | `assert` on a tuple literal (always true) | ✅ |
| **RAB078** | `except Exception: pass` — silent swallow | ❌ |
| **RAB079** | `__del__` method defined | ❌ |
| **RAB081** | `return`/`break`/`continue` inside `finally` | ❌ |
| **RAB082** | `except BaseException` too broad | ✅ |
| **RAB083** | Nested ternary expression | ❌ |
| **RAB084** | Bare `raise` outside an `except` block | ❌ |
| **RAB099** | Duplicate key/element in dict/set literal | ❌ |
| **RAB100** | Redundant `elif` after `return`/`raise`/`break`/`continue` | ❌ |
| **RAB103** | Self-comparison (`x == x`) always True/False | ❌ |
| **RAB105** | Inconsistent return statements (mixed bare and valued) | ❌ |
| **RAB106** | Too broad `except Exception:` catch | ❌ |
| **RAB108** | `__all__` contains non-string elements | ❌ |
| **RAB113** | `eval()` / `exec()` detected (security risk) | ❌ |
| **RAB114** | `pickle.load()` / `pickle.loads()` on untrusted data | ❌ |
| **RAB115** | `yaml.load()` without `Loader=` (security risk) | ❌ |
| **RAB118** | `del` on exception variable (Python 3.12+ clears chain) | ❌ |
| **RAB119** | `__init__` in subclass missing `super().__init__()` | ❌ |
| **RAB120** | Modifying iterable during iteration | ❌ |
| **RAB123** | Deprecated `asyncio.get_event_loop()` / `ensure_future()` | ❌ |
| **RAB124** | Blocking call inside async function | ❌ |
| **RAB127** | Redundant `else` in loop with `break` | ❌ |
| **RAB128** | `not ... in` → `not in` (PEP 8) | ✅ |
| **RAB129** | f-string in logging call (eager evaluation) | ✅ |

### performance — Performance improvements

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB004** | `any([...])` / `all([...])` with list comprehension | ✅ |
| **RAB005** | `for k in d.keys()` instead of `for k in d` | ✅ |
| **RAB008** | `for i in range(len(x))` instead of `enumerate(x)` | ❌ |
| **RAB009** | String concatenation in a loop (`s += str(x)`) | ❌ |
| **RAB010** | `set(list(x))` — unnecessary `list()` call | ❌ |
| **RAB015** | `k in d.keys()` instead of `k in d` | ✅ |
| **RAB019** | `os.system()` instead of `subprocess.run()` | ❌ |
| **RAB020** | `time.time()` for benchmarking | ❌ |
| **RAB024** | Deep comprehension (> 2 nested `for` clauses) | ❌ |
| **RAB036** | `x**2` / `x**3` → `x * x` / `x * x * x` | ✅ |
| **RAB037** | `map(lambda, ...)` / `filter(lambda, ...)` | ✅ |
| **RAB038** | `x in [const, ...]` → `x in {const, ...}` | ✅ |
| **RAB040** | `@dataclass` without `slots=True` | ✅ |
| **RAB041** | `re.compile()` inside a function | ❌ |
| **RAB042** | `for line in f.readlines()` → `for line in f` | ✅ |
| **RAB043** | Manual `for` loop with `.append()` instead of comprehension | ❌ |
| **RAB046** | `sorted(x)[0]` → `min(x)` | ✅ |
| **RAB047** | `sorted(x)[-1]` → `max(x)` | ✅ |
| **RAB050** | `for i in range(len(seq))` — iterate directly | ❌ |
| **RAB051** | `d.setdefault(k, []).append(v)` → `defaultdict` | ❌ |
| **RAB056** | Nested `with` statements | ❌ |
| **RAB060** | `sorted(x).sort()` → `x.sort()` | ✅ |
| **RAB066** | Function definition inside a loop | ❌ |
| **RAB071** | List comp inside `str.join()` → generator | ✅ |
| **RAB085** | `reversed(sorted(x))` → `sorted(x, reverse=True)` | ✅ |
| **RAB087** | `{k: v for k, v in zip(...)}` → `dict(zip(...))` | ✅ |
| **RAB088** | `while len(x) > 0` → `while x` | ✅ |
| **RAB089** | `copy.copy(x)` → `x.copy()` | ✅ |
| **RAB104** | Pass-through generator `list(x for x in y)` → `list(y)` | ❌ |
| **RAB130** | `len([... for ...])` → `sum(1 for ...)` | ✅ |
| **RAB131** | `set([...])` / `tuple([...])` / `sorted([...])` → generator | ✅ |
| **RAB132** | `x = x + [..]` in a loop (O(n²)) → `.append()` / `.extend()` | ✅ |
| **RAB133** | `list.pop(0)` / `list.insert(0, …)` → `collections.deque` | ❌ |
| **RAB134** | `sorted(x)[:k]` / `sorted(x)[-k:]` → `heapq.nsmallest`/`nlargest` | ❌ |
| **RAB135** | `for … in list(range(…))` → iterate `range(…)` directly | ✅ |

### style — Code style and conventions

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB014** | `class Foo(object)` in Python 3 | ❌ |
| **RAB016** | `.format()` instead of f-string | ❌ |
| **RAB023** | Redundant `.call()` method call | ❌ |
| **RAB026** | `sorted(list(x))` / `reversed(tuple(x))` — redundant collection | ✅ |
| **RAB032** | `bool(x)` inside a boolean context | ✅ |
| **RAB039** | `List[X]` / `Dict[K,V]` → `list[X]` / `dict[K,V]` | ✅ |
| **RAB044** | `Optional[X]` / `Union[A, B]` → `X \| None` / `A \| B` | ✅ |
| **RAB049** | `x = x + 1` → `x += 1` | ✅ |
| **RAB055** | Unused loop variable → `_` | ✅ |
| **RAB058** | `return True if cond else False` → `return cond` | ✅ |
| **RAB062** | Redundant `pass` after docstring | ✅ |
| **RAB067** | Variable/function shadows built-in name | ✅ |
| **RAB069** | `dict()` / `list()` / `tuple()` → `{}` / `[]` / `()` | ✅ |
| **RAB073** | Old-style `%` string formatting | ❌ |
| **RAB074** | `os.path.*` → `pathlib.Path` | ❌ |
| **RAB080** | `list(d.keys())` / `list(d.values())` → `list(d)` | ✅ |
| **RAB086** | `x == a or x == b` → `x in (a, b)` | ✅ |
| **RAB107** | TODO/FIXME/HACK/XXX comment left in code | ❌ |
| **RAB109** | Class name should use CamelCase convention | ❌ |
| **RAB110** | Function name should use snake_case convention | ❌ |
| **RAB111** | Module-level constant should use UPPER_CASE naming | ❌ |
| **RAB112** | Unnecessary `pass` in non-empty body | ✅ |
| **RAB126** | Magic number literal, assign to named constant | ❌ |

### typesafety — Type annotation rules

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB022** | Public function missing return type hint | ❌ |
| **RAB090** | Public function parameter missing type annotation | ❌ |
| **RAB091** | Function missing return type annotation | ❌ |
| **RAB092** | Class attribute missing type annotation | ❌ |
| **RAB093** | Module-level variable missing type annotation | ❌ |
| **RAB094** | `Any` type annotation used, prefer concrete type | ❌ |
| **RAB095** | Default value incompatible with type annotation | ❌ |

### complexity — Code complexity metrics

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB017** | Function too long (> 30 statements) | ❌ |
| **RAB018** | Too many parameters (> 6) | ❌ |
| **RAB101** | Cyclomatic complexity > 10 | ❌ |
| **RAB102** | Cognitive complexity > 15 | ❌ |

### import — Import-related rules

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB061** | `from module import *` — wildcard import | ❌ |
| **RAB096** | Unused import | ❌ |
| **RAB098** | Import inside function/class body, move to module level | ❌ |

### unused — Unused variables and imports

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB001** | Variable assigned but never used | ❌ |

---

## Runtime profiler (2.0 preview)

Beyond static analysis, rabbitinspect ships a **sampling runtime profiler** written in Rust. A background thread snapshots the Python stack via `sys._current_frames()` (robust across CPython 3.10–3.14) and samples resident memory, with near-zero changes to your code. On stop it writes a **self-contained HTML report**.

```bash
# Profile a script and write an HTML report
rabbitinspect perf run myscript.py --out report.html

# Also export an interactive flamegraph for https://speedscope.app
rabbitinspect perf run myscript.py --speedscope profile.speedscope.json

# Attach to an already-running process — no code changes, no restart (Linux)
rabbitinspect perf attach --pid 12345 --duration 5 --out report.html

# Compare two runs (before / after an optimization)
rabbitinspect perf run myscript.py --json before.json
# … make your change …
rabbitinspect perf run myscript.py --json after.json
rabbitinspect perf diff before.json after.json --out diff.html
```

The report includes:

- **Top functions** by self/total time, an **inline flamegraph**, and a **memory-over-time** chart.
- **On-CPU vs off-CPU** split (attach mode): how much self time was real CPU work vs waiting on sleep / I/O / locks.
- **Request timeline** + per-endpoint **p50/p95/p99** (when web middleware is installed).
- **Database** section: slowest queries and **N+1 detection** (repeated query shapes within one request).
- **Hotspots with lint findings**: the hottest functions cross-referenced against rabbitinspect's own static rules — the static perf rules pointed straight at the code that dominates runtime.

### Web frameworks

Drop-in middleware records one span per request (method, route, status, duration). Both are no-ops when the profiler is off, so they are safe to leave installed.

```python
# Django (wsgi.py)
from rabbitinspect.perf_web import WSGIProfilerMiddleware
application = WSGIProfilerMiddleware(application)

# FastAPI / Starlette
from rabbitinspect.perf_web import ASGIProfilerMiddleware
app.add_middleware(ASGIProfilerMiddleware)
```

For forked workers (gunicorn/uvicorn), call `enable_fork_profiling()` in the parent so each worker gets its own live sampler (OS threads don't survive `fork()`).

### Database queries

```python
from rabbitinspect.perf_db import instrument_sqlalchemy, instrument_django, query_timer

instrument_sqlalchemy(engine)   # SQLAlchemy
instrument_django()             # Django (call once at startup)

with query_timer('SELECT ...'):  # generic
    cursor.execute('SELECT ...')
```

> Timings are **statistical** (sampling), not exact per-call. Attach mode is Linux-only and supports CPython 3.13+ (validated on 3.13 and 3.14; 3.11/3.12 predate CPython's remote-debug offsets). Per-function memory attribution is on the roadmap.

---

## Performance rationale

### Why Rust?

Parsing and analyzing Python source is a CPU-bound operation. Rust's zero-cost abstractions and lack of a garbage collector make it ideal for this kind of static analysis. The core engine uses `rustpython-parser` to build a full AST and walks it with pattern matching — all without any Python overhead at analysis time.

### Specific improvements

| Pattern | Inefficiency | Fix |
|---------|-------------|-----|
| `x == None` | Calls `x.__eq__(None)`, may be overridden | `x is None` — direct pointer comparison |
| `any([x for x in items])` | Builds full list in memory | `any(x for x in items)` — generator yields one at a time |
| `len(x) == 0` | Calls `x.__len__()`, may be O(n) | `not x` — uses `__bool__` protocol |
| `for k in d.keys()` | Creates a `dict_keys` view object | `for k in d:` — iterates dict directly |
| `type(x) == T` | Ignores subclass relationships | `isinstance(x, T)` — correct for inheritance |
| `for i in range(len(x))` | Double lookup `x[i]` overhead | `for i, item in enumerate(x)` |
| `s += str(x)` in loop | O(n²) string allocations | `"".join(...)` — single allocation |
| `set(list(x))` | Intermediate list allocation | `set(x)` — builds directly from iterator |
| `k in d.keys()` | Creates view for membership test | `k in d` — direct hash lookup |
| `sorted(list(x))` | Intermediate list before sort | `sorted(x)` — sorts iterable directly |
| `if k in d: return d[k]` | Two dict lookups | `return d.get(k)` — single lookup |
| `x[:]` for list copy | Slice intent unclear | `x.copy()` — explicit intent |
| `sorted(x)[0]` | O(n log n) sort for min | `min(x)` — O(n) single pass |
| `sorted(x)[-1]` | O(n log n) sort for max | `max(x)` — O(n) single pass |
| `List[int]` / `Dict[str, int]` | Requires `typing` import | `list[int]` / `dict[str, int]` — Python 3.9+ |
| `for line in f.readlines()` | Loads entire file into memory | `for line in f` — line by line, O(1) memory |
| `d.setdefault(k, []).append(v)` | Creates default on every call | `defaultdict(list)` — default only on missing key |
| `type(x) == A or type(x) == B` | Multiple `type()` calls | `isinstance(x, (A, B))` — single check |
| `reversed(sorted(x))` | Two passes over data | `sorted(x, reverse=True)` — single pass |
| `{k: v for k, v in zip(...)}` | Comprehension overhead | `dict(zip(...))` — direct constructor |

---

## Development

```bash
# Setup
git clone https://github.com/rroblf01/rabbitinspect
cd rabbitinspect

# Build the Rust extension (requires Rust toolchain)
cargo build --release
cp target/release/librabbitinspect.so python/rabbitinspect/_core.cpython-314-x86_64-linux-gnu.so

# Run tests
uv run pytest tests/

# Build distributable wheels
uv run maturin build
```

---

## License

MIT
