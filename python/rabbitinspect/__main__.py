import argparse
import json
import os
import pathlib
import re
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # Python 3.10 fallback

from rabbitinspect import analyze_code, apply_fixes


def parse_pyproject_toml(path: str | None = None) -> dict:
    config: dict = {}
    search_dir = pathlib.Path(path or os.getcwd()).resolve()
    for parent in [search_dir] + list(search_dir.parents):
        candidate = parent / "pyproject.toml"
        if candidate.is_file():
            try:
                with open(candidate, "rb") as f:
                    data = tomllib.load(f)
                config = data.get("tool", {}).get("rabbitinspect", {})
            except (tomllib.TOMLDecodeError, OSError):
                pass
            break
    return config


def parse_noqa(source: str) -> dict[int, set[str]]:
    noqa_map: dict[int, set[str]] = {}
    pattern = re.compile(r"#\s*noqa(?:\s*:\s*(\S+))?\s*$")
    for i, line in enumerate(source.splitlines(), start=1):
        m = pattern.search(line)
        if m:
            raw = m.group(1)
            if raw:
                codes = {c.strip() for c in raw.split(",") if c.strip()}
                noqa_map[i] = codes
            else:
                noqa_map[i] = set()  # suppress all
    return noqa_map


def filter_findings(
    findings: list[dict],
    source: str,
    select: set[str] | None,
    ignore: set[str] | None,
) -> list[dict]:
    noqa_map = parse_noqa(source)

    filtered: list[dict] = []
    for f in findings:
        code = f["code"]
        line = f["line"]

        if select is not None and code not in select:
            continue
        if ignore is not None and code in ignore:
            continue

        noqa_codes = noqa_map.get(line)
        if noqa_codes is not None:
            if not noqa_codes or code in noqa_codes:
                continue

        filtered.append(f)
    return filtered


def format_finding(file_path: str, finding: dict) -> str:
    return (
        f"{file_path}:{finding['line']}:{finding['col']}: "
        f"{finding['code']} {finding['message']}"
    )


def collect_python_files(paths: list[str]) -> list[str]:
    files: list[str] = []
    for p in paths:
        pobj = pathlib.Path(p)
        if pobj.is_file():
            if pobj.suffix == ".py":
                files.append(str(pobj.resolve()))
        elif pobj.is_dir():
            for py_file in sorted(pobj.rglob("*.py")):
                files.append(str(py_file.resolve()))
    return files


EXPLANATIONS: dict[str, str] = {
    "RAB001": "Variable is assigned but never used. Assigning to a variable that is never read "
               "suggests dead code or a logic error. Remove the assignment or use it.",
    "RAB002": "Use 'x is None' instead of 'x == None'. The equality operator (==) calls "
               "__eq__() which can be overridden, while 'is' compares identity directly.",
    "RAB003": "Use 'not x' instead of 'len(x) == 0'. Python objects define __bool__() for "
               "truthiness checking, which is both faster and more idiomatic.",
    "RAB004": "Use a generator expression instead of a list comprehension inside any(), all(), "
               "sum(), min(), or max(). The list comprehension builds a full list in memory "
               "while a generator yields items one at a time.",
    "RAB005": "Use 'for k in d' instead of 'for k in d.keys()'. Iterating over a dict directly "
               "is equivalent and avoids creating an intermediate view object.",
    "RAB006": "Use isinstance(x, T) instead of type(x) == T. type() comparisons ignore subclass "
               "relationships and are fragile with inheritance.",
    "RAB007": "Unnecessary 'else' after return/raise/break/continue. The control flow after "
               "a terminal statement makes the else unreachable; dedent the else body.",
    "RAB008": "Use enumerate() instead of range(len()). enumerate() is more readable and "
               "avoids double lookups via indexing.",
    "RAB009": "String concatenation in a loop (s += str(x)) creates O(n^2) allocations. "
               "Use a list and ''.join() for linear performance.",
    "RAB010": "set(list(x)) can be simplified to set(x). The intermediate list is unnecessary.",
    "RAB011": "Mutable default arguments (list, dict, set) are shared across all calls. "
               "Use None and initialize inside the function instead.",
    "RAB012": "Bare 'except:' catches all exceptions including SystemExit and KeyboardInterrupt. "
               "Always specify the exception type.",
    "RAB013": "Bare 'except: pass' silently swallows all exceptions including SystemExit and "
               "KeyboardInterrupt. At minimum, log the error.",
    "RAB014": "class Foo(object) is redundant in Python 3. Use 'class Foo:' directly.",
    "RAB015": "Use 'k in d' instead of 'k in d.keys()'. The 'in' operator on a dict already "
               "checks keys directly without creating a view.",
    "RAB016": "Use f-strings instead of .format() for better readability and performance.",
    "RAB017": "Function is too long (over 30 statements). Consider refactoring into smaller "
               "helper functions for readability and testability.",
    "RAB018": "Too many function parameters (over 6). Consider using a dataclass or splitting "
               "the function.",
    "RAB019": "Use subprocess.run() instead of os.system() for better control, error handling, "
               "and security.",
    "RAB020": "Use time.perf_counter() instead of time.time() for benchmarking. time.time() "
               "has lower resolution and can jump (NTP adjustments).",
    "RAB022": "Public function is missing a return type hint. Adding return types improves "
               "documentation and enables static type checking.",
    "RAB023": "Redundant .call() method. Call the object directly instead.",
    "RAB024": "Deep comprehension with more than 2 nested 'for' clauses. Consider refactoring "
               "into helper loops for readability.",
    "RAB025": "Long if-elif chain with more than 3 branches. Consider using a dict dispatch.",
    "RAB026": "sorted(list(x)) has a redundant list() call. Use sorted(x) directly.",
    "RAB029": "Use 'x' or 'not x' instead of comparing to True/False with ==. Boolean values "
               "are already truthy/falsy.",
    "RAB030": "Unnecessary if/else returning boolean literals. Replace with 'return <cond>'.",
    "RAB031": "Use d.get(k) instead of 'if k in d: return d[k]' to avoid a double dict lookup.",
    "RAB032": "Unnecessary bool() call in a boolean context. Use the value directly.",
    "RAB034": "assert True is always a no-op; assert False always fails. Use "
               "raise AssertionError instead of assert False.",
    "RAB035": "Use x.copy() instead of x[:] for list copies. .copy() is more explicit.",
    "RAB036": "Use x * x instead of x**2, and x * x * x instead of x**3 for performance.",
    "RAB037": "Use a generator expression instead of map()/filter() with a lambda for "
               "better readability.",
    "RAB038": "Use 'x in {const, ...}' instead of 'x in [const, ...]' for O(1) membership.",
    "RAB039": "Use native generics (list[...], dict[...]) instead of typing.List, typing.Dict "
               "(Python 3.9+).",
    "RAB040": "Use @dataclass(slots=True) to reduce memory usage and speed attribute access.",
    "RAB041": "Move re.compile() to module level to avoid recompiling the regex on every call.",
    "RAB042": "Use 'for line in f' instead of 'for line in f.readlines()' to avoid loading "
               "the entire file into memory.",
    "RAB043": "Use a list comprehension instead of a manual for loop with .append().",
    "RAB044": "Use X | None instead of Optional[X], and A | B instead of Union[A, B] "
               "(Python 3.10+).",
    "RAB045": "Use 'with open(...) as f:' to ensure the file is properly closed.",
    "RAB046": "Use min(x) instead of sorted(x)[0] for O(n) minimum.",
    "RAB047": "Use max(x) instead of sorted(x)[-1] for O(n) maximum.",
    "RAB048": "Use 'x is not None' instead of 'not x is None' (PEP 8 style).",
    "RAB049": "Use augmented assignment (x += 1) instead of (x = x + 1).",
    "RAB050": "Iterate directly over the sequence instead of using range(len(...)).",
    "RAB051": "Use collections.defaultdict(list) instead of setdefault(..., []).append().",
    "RAB052": "Use isinstance(x, (A, B)) instead of multiple 'type(x) == A or type(x) == B'.",
    "RAB053": "Use 'x' instead of 'x is True', and 'not x' instead of 'x is False'.",
    "RAB054": "Use 'x = x or y' instead of 'if not x: x = y'.",
    "RAB055": "Unused loop variable. Replace with '_' to indicate it's intentionally unused.",
    "RAB056": "Nested 'with' statements can be combined into a single with statement.",
    "RAB057": "Use s.startswith(('a', 'b')) instead of repeated startswith() calls.",
    "RAB058": "Use 'return <cond>' instead of 'return True if cond else False'.",
    "RAB059": "while True: without a break statement results in an infinite loop.",
    "RAB060": "Use x.sort() instead of sorted(x).sort(). sorted() creates a new list.",
    "RAB061": "Wildcard imports pollute the namespace. Import specific names instead.",
    "RAB062": "Redundant 'pass' after a docstring can be removed.",
    "RAB063": "Use '==' instead of 'is' to compare with literals. 'is' is for identity, "
               "not value equality.",
    "RAB064": "__init__ should not return a value. Use bare 'return' instead.",
    "RAB065": "if True: is always true; if False: is always false. Remove the condition.",
    "RAB066": "Function defined inside a loop is recreated on every iteration. "
               "Move it outside the loop.",
    "RAB067": "Variable/function shadows a Python built-in name. Rename to 'name_' to avoid "
               "confusion.",
    "RAB068": "Raise inside 'except' without 'from' loses the original exception traceback.",
    "RAB069": "Use literal syntax ({} / [] / ()) instead of dict()/list()/tuple() calls.",
    "RAB070": "Use 'not x' instead of comparing to an empty literal.",
    "RAB071": "Use a generator expression instead of a list comprehension inside str.join().",
    "RAB072": "Except handler only re-raises the exception. Remove the handler entirely.",
    "RAB073": "Use f-strings instead of old-style % string formatting.",
    "RAB074": "Use pathlib.Path instead of os.path functions for modern path handling.",
    "RAB075": "Use isinstance(x, A) instead of isinstance(x, (A,)) for a single type.",
    "RAB076": "Redundant str() call on a value that is already a string.",
    "RAB078": "except Exception: pass silently swallows all exceptions. Log the error or "
               "handle it properly.",
    "RAB079": "__del__ method defined. Use a context manager or explicit cleanup instead.",
    "RAB080": "Use list(d) instead of list(d.keys()) / list(d.values()).",
    "RAB083": "Nested ternary expression harms readability. Use if/elif/else instead.",
    "RAB085": "Use sorted(x, reverse=True) instead of reversed(sorted(x)).",
    "RAB087": "Use dict(zip(...)) instead of a dict comprehension over zip().",
    "RAB088": "Use 'while x:' instead of 'while len(x) > 0:'.",
    "RAB089": "Use x.copy() instead of copy.copy(x) for lists and dicts.",
    "RAB090": "Public function parameter is missing a type annotation. Add types for better "
               "documentation and type checking.",
    "RAB091": "Function is missing a return type annotation. Add ': -> ReturnType' for "
               "better documentation and type checking.",
    "RAB092": "Class attribute is missing a type annotation. Add ': Type' to document "
               "the expected type.",
    "RAB093": "Module-level variable is missing a type annotation. Add ': Type' for "
               "better documentation.",
    "RAB094": "Any type annotation is too broad. Use a more specific type when possible.",
    "RAB095": "Default value may be incompatible with the declared type annotation. "
               "For example, 'x: str = None' should be 'x: str | None = None'.",
    "RAB096": "Import is unused. Remove unused imports to keep the code clean.",
    "RAB097": "Debugging call (print/breakpoint/pdb.set_trace) left in production code. "
               "Remove before committing.",
    "RAB098": "Import inside a function or class body should be moved to module level "
               "(PEP 8 recommends imports at the top of the file).",
    "RAB099": "Duplicate key/element in dict/set literal. The later value overwrites the "
               "earlier one, which is likely a bug.",
    "RAB100": "Redundant 'elif' after return/raise/break/continue. The terminal statement "
               "makes the elif unreachable.",
    "RAB101": "Cyclomatic complexity exceeds the threshold. Consider simplifying the "
               "function by splitting it up.",
    "RAB102": "Cognitive complexity exceeds the threshold. The function is hard to "
               "understand — consider refactoring.",
    "RAB103": "Self-comparison (e.g., 'x == x') always evaluates to True (or False for "
               "'!='). This is likely a typo.",
    "RAB104": "Pass-through generator (list(x for x in y)) can be simplified to list(y).",
    "RAB105": "Inconsistent return statements: mix of bare 'return' and 'return <value>' "
               "in the same function. Some paths return None implicitly, others return a value.",
    "RAB106": "Too broad 'except Exception:' catches too many errors. Catch only the "
               "specific exceptions you expect.",
    "RAB107": "Comment contains TODO/FIXME/HACK/XXX marker. Address the issue before "
               "shipping.",
    "RAB108": "__all__ should contain only string literals. Variables in __all__ may not "
               "resolve correctly.",
    "RAB109": "Class name should use CamelCase convention (PEP 8).",
    "RAB110": "Function name should use snake_case convention (PEP 8).",
    "RAB111": "Module-level constant assigned a literal value should use UPPER_CASE "
               "naming (PEP 8).",
    "RAB112": "Unnecessary 'pass' in a non-empty body. Remove the redundant pass statement.",
    "RAB113": "eval() and exec() execute arbitrary code and are security risks. Avoid "
               "them unless absolutely necessary.",
    "RAB114": "pickle.load() / pickle.loads() on untrusted data can execute arbitrary code. "
               "Use a safer serialization format like JSON.",
    "RAB115": "yaml.load() without Loader= allows arbitrary code execution. Use "
               "yaml.safe_load() or specify Loader=yaml.SafeLoader.",
    "RAB118": "Deleting an exception variable with 'del' inside an except block clears the "
               "exception chain in Python 3.12+, potentially losing traceback information.",
    "RAB119": "__init__ in a subclass does not call super().__init__(). The parent class "
               "may not initialize properly.",
    "RAB120": "Modifying a list/dict while iterating over it can cause skipped items, "
               "duplicate processing, or runtime errors. Copy the iterable first.",
    "RAB123": "asyncio.get_event_loop() is deprecated in Python 3.12+. Use "
               "asyncio.get_running_loop() or asyncio.new_event_loop(). "
               "asyncio.ensure_future() is deprecated; use asyncio.create_task().",
    "RAB124": "Blocking call inside async function blocks the event loop. Use async "
               "alternatives (e.g., asyncio.sleep() instead of time.sleep()).",
    "RAB126": "Magic number literal. Assign the value to a named constant to clarify "
               "its meaning.",
    "RAB127": "Loop has both 'break' and 'else' clause. The 'else' runs only if no break "
               "occurs; if the else is always reached, remove the break.",
    "RAB128": "Use 'x not in y' instead of 'not x in y' (PEP 8 style).",
}

ALL_CODES = sorted(EXPLANATIONS.keys())


def print_code_list(color: bool) -> None:
    for code in ALL_CODES:
        desc = EXPLANATIONS[code].split(".")[0].strip()
        has_fix = "✅" if code in (
            "RAB002", "RAB003", "RAB004", "RAB005", "RAB006", "RAB007",
            "RAB015", "RAB026", "RAB029", "RAB030", "RAB031", "RAB032",
            "RAB034", "RAB035", "RAB036", "RAB037", "RAB038", "RAB039",
            "RAB040", "RAB042", "RAB044", "RAB046", "RAB047", "RAB048",
            "RAB049", "RAB052", "RAB054", "RAB055", "RAB057", "RAB058",
            "RAB060", "RAB062", "RAB063", "RAB064", "RAB067", "RAB069",
            "RAB070", "RAB071", "RAB075", "RAB076", "RAB080", "RAB085",
            "RAB087", "RAB088", "RAB089", "RAB112", "RAB128",
        ) else "❌"
        line = f"{code}  {desc:<65} {has_fix}"
        if color:
            line = f"\x1b[36m{code}\x1b[0m  {desc:<65} {has_fix}"
        print(line)


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="rabbitinspect",
        description="Python code performance analyzer — detects improvements in speed and efficiency",
    )
    parser.add_argument(
        "paths",
        nargs="*",
        help="Python file(s) or director(ies) to analyze",
    )
    parser.add_argument(
        "--fix",
        action="store_true",
        help="Apply auto-fixes for selected checks",
    )
    parser.add_argument(
        "--no-color",
        action="store_true",
        help="Disable colored output",
    )
    parser.add_argument(
        "--select",
        type=str,
        help="Comma-separated list of check codes to enable (e.g. 'RAB002,RAB003')",
    )
    parser.add_argument(
        "--ignore",
        type=str,
        help="Comma-separated list of check codes to disable (e.g. 'RAB022,RAB101')",
    )
    parser.add_argument(
        "--format",
        type=str,
        choices=["text", "json"],
        default="text",
        help="Output format (default: text)",
    )
    parser.add_argument(
        "--explain",
        type=str,
        metavar="CODE",
        help="Show detailed explanation for a check code (e.g. 'RAB002')",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        dest="show_list",
        help="List all available check codes with descriptions",
    )

    cli_args = parser.parse_args()

    use_color = not cli_args.no_color and sys.stdout.isatty()

    if cli_args.explain:
        code = cli_args.explain.strip().upper()
        if code in EXPLANATIONS:
            print(f"\x1b[1m{code}\x1b[0m" if use_color else code)
            print("-" * len(code))
            print(EXPLANATIONS[code])
        else:
            print(f"Unknown check code: {code}", file=sys.stderr)
            sys.exit(1)
        return

    if cli_args.show_list:
        print_code_list(use_color)
        return

    if not cli_args.paths:
        parser.print_help()
        sys.exit(1)

    files = collect_python_files(cli_args.paths)

    if not files:
        print("No Python files found.", file=sys.stderr)
        sys.exit(1)

    config = parse_pyproject_toml()
    select_codes: set[str] | None = None
    ignore_codes: set[str] | None = None

    if cli_args.select:
        select_codes = {c.strip().upper() for c in cli_args.select.split(",") if c.strip()}
    elif "select" in config:
        select_codes = {c.strip().upper() for c in config["select"]}

    if cli_args.ignore:
        ignore_codes = {c.strip().upper() for c in cli_args.ignore.split(",") if c.strip()}
    elif "ignore" in config:
        ignore_codes = {c.strip().upper() for c in config["ignore"]}

    use_color = not cli_args.no_color and sys.stdout.isatty()
    all_results: list[dict] = []
    total_findings = 0
    total_fixed = 0

    for file_path in files:
        try:
            with open(file_path, encoding="utf-8") as f:
                source = f.read()
        except (OSError, UnicodeDecodeError) as e:
            print(f"Error reading {file_path}: {e}", file=sys.stderr)
            continue

        raw_findings = analyze_code(source)
        findings = filter_findings(raw_findings, source, select_codes, ignore_codes)

        if not findings:
            continue

        total_findings += len(findings)
        fixable = [f for f in findings if f.get("fix")]

        if cli_args.format == "json":
            for f in findings:
                all_results.append({
                    "file": file_path,
                    "line": f["line"],
                    "col": f["col"],
                    "end_line": f["end_line"],
                    "end_col": f["end_col"],
                    "code": f["code"],
                    "message": f["message"],
                })
        else:
            for finding in findings:
                line = format_finding(file_path, finding)
                if use_color:
                    if finding.get("fix"):
                        line = f"\x1b[33m{line}\x1b[0m"
                    else:
                        line = f"\x1b[36m{line}\x1b[0m"
                print(line)

        if cli_args.fix and fixable:
            raw_fixes = [f["fix"] for f in fixable]
            new_source = apply_fixes(source, raw_fixes)
            try:
                with open(file_path, "w", encoding="utf-8") as f:
                    f.write(new_source)
                total_fixed += len(fixable)
                if use_color:
                    print(f"\x1b[32m  Fixed {len(fixable)} issue(s) in {file_path}\x1b[0m")
                else:
                    print(f"  Fixed {len(fixable)} issue(s) in {file_path}")
            except OSError as e:
                print(f"Error writing {file_path}: {e}", file=sys.stderr)

    if cli_args.format == "json":
        print(json.dumps(all_results, indent=2))
    elif total_findings == 0:
        msg = "✓ No issues found — your code looks clean!"
        if use_color:
            msg = f"\x1b[32m{msg}\x1b[0m"
        print(msg)
    else:
        summary = f"Found {total_findings} issue(s)"
        if total_fixed:
            summary += f", fixed {total_fixed}"
        summary += f" in {len([f for f in files if os.path.exists(f)])} file(s)"
        if use_color:
            summary = f"\x1b[33m{summary}\x1b[0m"
        print(summary)

    if total_findings > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
