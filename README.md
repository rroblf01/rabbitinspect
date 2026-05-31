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
| `rabbitinspect --list` | List all available checks with descriptions |
| `rabbitinspect --select RAB002,RAB003 file.py` | Run only specific checks |
| `rabbitinspect --ignore RAB022,RAB101 file.py` | Skip specific checks |
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

## Checks

| Code | Check | Auto-fix |
|------|-------|----------|
| **RAB001** | Variable assigned but never used | ❌ |
| **RAB002** | `x == None` instead of `x is None` | ✅ |
| **RAB003** | `len(x) == 0` instead of `not x` | ✅ |
| **RAB004** | `any([...])` / `all([...])` with list comprehension | ✅ |
| **RAB005** | `for k in d.keys()` instead of `for k in d` | ✅ |
| **RAB006** | `type(x) == T` instead of `isinstance(x, T)` | ✅ |
| **RAB007** | Unnecessary `else` after `return`/`raise`/`break`/`continue` | ✅ |
| **RAB008** | `for i in range(len(x))` instead of `enumerate(x)` | ❌ |
| **RAB009** | String concatenation in a loop (`s += str(x)`) | ❌ |
| **RAB010** | `set(list(x))` — unnecessary `list()` call | ❌ |
| **RAB011** | Mutable default argument (`x=[]`, `x={}`) | ❌ |
| **RAB012** | Bare `except:` clause | ❌ |
| **RAB013** | Bare `except: pass` | ❌ |
| **RAB014** | `class Foo(object)` in Python 3 | ❌ |
| **RAB015** | `k in d.keys()` instead of `k in d` | ✅ |
| **RAB016** | `.format()` instead of f-string | ❌ |
| **RAB017** | Function too long (> 30 statements) | ❌ |
| **RAB018** | Too many parameters (> 6) | ❌ |
| **RAB019** | `os.system()` instead of `subprocess.run()` | ❌ |
| **RAB020** | `time.time()` for benchmarking | ❌ |
| **RAB022** | Public function missing return type hint | ❌ |
| **RAB023** | Redundant `.call()` method call | ❌ |
| **RAB024** | Deep comprehension (> 2 nested `for` clauses) | ❌ |
| **RAB025** | Long if-elif chain (> 3 branches) | ❌ |
| **RAB026** | `sorted(list(x))` / `reversed(tuple(x))` — redundant collection | ✅ |
| **RAB029** | `x == True` / `x == False` instead of `x` / `not x` | ✅ |
| **RAB030** | `if cond: return True else: return False` → `return cond` | ✅ |
| **RAB031** | `if k in d: return d[k]` → `d.get(k)` | ✅ |
| **RAB032** | `bool(x)` inside a boolean context | ✅ |
| **RAB034** | `assert True` / `assert False` | ✅ |
| **RAB035** | `x[:]` → `x.copy()` for list copies | ✅ |
| **RAB036** | `x**2` / `x**3` → `x * x` / `x * x * x` | ✅ |
| **RAB037** | `map(lambda, ...)` / `filter(lambda, ...)` | ✅ |
| **RAB038** | `x in [const, ...]` → `x in {const, ...}` | ✅ |
| **RAB039** | `List[X]` / `Dict[K,V]` → `list[X]` / `dict[K,V]` | ✅ |
| **RAB040** | `@dataclass` without `slots=True` | ✅ |
| **RAB041** | `re.compile()` inside a function | ❌ |
| **RAB042** | `for line in f.readlines()` → `for line in f` | ✅ |
| **RAB043** | Manual `for` loop with `.append()` instead of comprehension | ❌ |
| **RAB044** | `Optional[X]` / `Union[A, B]` → `X \| None` / `A \| B` | ✅ |
| **RAB045** | `open()` without context manager (`with`) | ❌ |
| **RAB046** | `sorted(x)[0]` → `min(x)` | ✅ |
| **RAB047** | `sorted(x)[-1]` → `max(x)` | ✅ |
| **RAB048** | `not x is None` → `x is not None` | ✅ |
| **RAB049** | `x = x + 1` → `x += 1` | ✅ |
| **RAB050** | `for i in range(len(seq))` — iterate directly | ❌ |
| **RAB051** | `d.setdefault(k, []).append(v)` → `defaultdict` | ❌ |
| **RAB052** | `type(x) == A or type(x) == B` → `isinstance(x, (A, B))` | ✅ |
| **RAB053** | `x is True` / `x is False` → `x` / `not x` | ✅ |
| **RAB054** | `if not x: x = y` → `x = x or y` | ✅ |
| **RAB055** | Unused loop variable → `_` | ✅ |
| **RAB056** | Nested `with` statements | ❌ |
| **RAB057** | `s.startswith('a') or s.startswith('b')` → `s.startswith(('a', 'b'))` | ✅ |
| **RAB058** | `return True if cond else False` → `return cond` | ✅ |
| **RAB059** | `while True:` without `break` → infinite loop | ❌ |
| **RAB060** | `sorted(x).sort()` → `x.sort()` | ✅ |
| **RAB061** | `from module import *` — wildcard import | ❌ |
| **RAB062** | Redundant `pass` after docstring | ✅ |
| **RAB063** | `x is 5` / `x is "str"` → `x == 5` / `x == "str"` | ✅ |
| **RAB064** | `__init__` returning non-None value | ✅ |
| **RAB065** | `if True:` / `if False:` dead code | ❌ |
| **RAB066** | Function definition inside a loop | ❌ |
| **RAB067** | Variable/function shadows built-in name | ✅ |
| **RAB068** | `raise Exc()` without `from` inside `except` | ❌ |
| **RAB069** | `dict()` / `list()` / `tuple()` → `{}` / `[]` / `()` | ✅ |
| **RAB070** | `x == ""` / `x == []` / `x == {}` → `not x` / `x` | ✅ |
| **RAB071** | List comp inside `str.join()` → generator | ✅ |
| **RAB072** | Except handler only re-raises | ❌ |
| **RAB073** | Old-style `%` string formatting | ❌ |
| **RAB074** | `os.path.*` → `pathlib.Path` | ❌ |
| **RAB075** | `isinstance(x, (A,))` → `isinstance(x, A)` | ✅ |
| **RAB076** | `str()` on value already a string | ✅ |
| **RAB078** | `except Exception: pass` — silent swallow | ❌ |
| **RAB079** | `__del__` method defined | ❌ |
| **RAB080** | `list(d.keys())` / `list(d.values())` → `list(d)` | ✅ |
| **RAB083** | Nested ternary expression | ❌ |
| **RAB085** | `reversed(sorted(x))` → `sorted(x, reverse=True)` | ✅ |
| **RAB087** | `{k: v for k, v in zip(...)}` → `dict(zip(...))` | ✅ |
| **RAB088** | `while len(x) > 0` → `while x` | ✅ |
| **RAB089** | `copy.copy(x)` → `x.copy()` | ✅ |
| **RAB090** | Public function parameter missing type annotation | ❌ |
| **RAB091** | Function missing return type annotation | ❌ |
| **RAB092** | Class attribute missing type annotation | ❌ |
| **RAB093** | Module-level variable missing type annotation | ❌ |
| **RAB094** | `Any` type annotation used, prefer concrete type | ❌ |
| **RAB095** | Default value incompatible with type annotation | ❌ |
| **RAB096** | Unused import | ❌ |
| **RAB097** | Debugging `print()` / `breakpoint()` / `pdb.set_trace()` left in code | ❌ |
| **RAB098** | Import inside function/class body, move to module level | ❌ |
| **RAB099** | Duplicate key/element in dict/set literal | ❌ |
| **RAB100** | Redundant `elif` after `return`/`raise`/`break`/`continue` | ❌ |
| **RAB103** | Self-comparison (`x == x`) always True/False | ❌ |
| **RAB104** | Pass-through generator `list(x for x in y)` → `list(y)` | ❌ |
| **RAB105** | Inconsistent return statements (mixed bare and valued) | ❌ |
| **RAB106** | Too broad `except Exception:` catch | ❌ |
| **RAB107** | TODO/FIXME/HACK/XXX comment left in code | ❌ |
| **RAB108** | `__all__` contains non-string elements | ❌ |
| **RAB109** | Class name should use CamelCase convention | ❌ |
| **RAB110** | Function name should use snake_case convention | ❌ |
| **RAB111** | Module-level constant should use UPPER_CASE naming | ❌ |
| **RAB112** | Unnecessary `pass` in non-empty body | ✅ |
| **RAB113** | `eval()` / `exec()` detected (security risk) | ❌ |
| **RAB114** | `pickle.load()` / `pickle.loads()` on untrusted data | ❌ |
| **RAB115** | `yaml.load()` without `Loader=` (security risk) | ❌ |
| **RAB118** | `del` on exception variable (Python 3.12+ clears chain) | ❌ |
| **RAB119** | `__init__` in subclass missing `super().__init__()` | ❌ |
| **RAB120** | Modifying iterable during iteration | ❌ |
| **RAB123** | Deprecated `asyncio.get_event_loop()` / `ensure_future()` | ❌ |
| **RAB124** | Blocking call inside async function | ❌ |
| **RAB126** | Magic number literal, assign to named constant | ❌ |
| **RAB127** | Redundant `else` in loop with `break` | ❌ |
| **RAB128** | `not ... in` → `not in` (PEP 8) | ✅ |
| **RAB101** | Cyclomatic complexity > 10 | ❌ |
| **RAB102** | Cognitive complexity > 15 | ❌ |

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
