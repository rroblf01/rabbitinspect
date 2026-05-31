import argparse
import json
import os
import re
import sys
import pathlib
import tomllib

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


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="rabbitinspect",
        description="Python code performance analyzer — detects improvements in speed and efficiency",
    )
    parser.add_argument(
        "paths",
        nargs="+",
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

    cli_args = parser.parse_args()
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
