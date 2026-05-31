import argparse
import os
import sys
import pathlib

from rabbitinspect import analyze_code, apply_fixes


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

    args = parser.parse_args()
    files = collect_python_files(args.paths)

    if not files:
        print("No Python files found.", file=sys.stderr)
        sys.exit(1)

    use_color = not args.no_color and sys.stdout.isatty()
    total_findings = 0
    total_fixed = 0

    for file_path in files:
        try:
            with open(file_path, encoding="utf-8") as f:
                source = f.read()
        except (OSError, UnicodeDecodeError) as e:
            print(f"Error reading {file_path}: {e}", file=sys.stderr)
            continue

        findings = analyze_code(source)

        if not findings:
            continue

        total_findings += len(findings)

        fixable = [f for f in findings if f.get("fix")]

        for finding in findings:
            line = format_finding(file_path, finding)
            if use_color:
                if finding.get("fix"):
                    line = f"\x1b[33m{line}\x1b[0m"
                else:
                    line = f"\x1b[36m{line}\x1b[0m"
            print(line)

        if args.fix and fixable:
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

    if total_findings == 0:
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
