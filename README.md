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
rabbitinspect check file.py

# Check an entire directory
rabbitinspect check src/

# Auto-fix detected issues
rabbitinspect check file.py --fix
```

Output (ruff-style):

```
file.py:10:5: RAB002 Use 'is' instead of '==' for None comparison
file.py:25:8: RAB003 Use 'not x' instead of 'len(x) == 0' for emptiness check
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

| Code | Check | Why it's faster | Auto-fix |
|------|-------|-----------------|----------|
| **RAB001** | Variable assigned but never used | Dead code wastes memory and confuses readers | ❌ (warning only) |
| **RAB002** | `x == None` instead of `x is None` | `is` avoids calling `__eq__` and is constant-time; `None` is a singleton | ✅ |
| **RAB003** | `len(x) == 0` instead of `not x` | `not x` is O(1) for most types and uses the existing truthiness protocol | ✅ |
| **RAB004** | `any([...])` / `all([...])` with list comprehension | Generator `(...)` avoids allocating the entire list in memory before iterating | ✅ |
| **RAB005** | `for k in d.keys()` instead of `for k in d` | Iterating the dict directly avoids creating a `.keys()` view object | ❌ (warning only) |
| **RAB006** | `type(x) == T` instead of `isinstance(x, T)` | `isinstance` handles inheritance and is the idiomatic Python way | ✅ |
| **RAB007** | Unnecessary `else` after `return`/`raise`/`break`/`continue` | Removing dead `else` reduces indentation and clarifies control flow | ❌ (warning only) |

---

## Performance rationale

### Why Rust?

Parsing and analyzing Python source is a CPU-bound operation. Rust's zero-cost abstractions and lack of a garbage collector make it ideal for this kind of static analysis. The core engine uses `rustpython-parser` to build a full AST and walks it with pattern matching — all without any Python overhead at analysis time.

### Specific improvements

| Pattern | Inefficiency | Fix |
|---------|-------------|-----|
| `x == None` | Calls `x.__eq__(None)`, which may be overridden; requires attribute lookup | `x is None` — direct pointer comparison, no method call |
| `any([x for x in items])` | Builds a full list in memory, then iterates it | `any(x for x in items)` — generator yields one element at a time |
| `len(x) == 0` | Calls `x.__len__()`, may be O(n) for some types; requires function call | `not x` — uses `__bool__` / `__len__` protocol directly, often O(1) |
| `for k in d.keys()` | Creates a `dict_keys` view object (small overhead) | `for k in d:` — iterates the dict directly |
| `type(x) == T` | Ignores subclass relationships; fails for derived types | `isinstance(x, T)` — correct for inheritance |

---

## Development

```bash
# Setup
git clone https://github.com/your-org/rabbitinspect
cd rabbitinspect

# Build the Rust extension (requires Rust toolchain)
uv run maturin develop

# Run tests
uv run pytest tests/

# Build distributable wheels
uv run maturin build
```

---

## License

MIT
