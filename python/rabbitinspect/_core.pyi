# ruff: noqa: I001
from typing import Any


def analyze_code(source: str) -> list[dict[str, Any]]: ...


def apply_fixes(source: str, fixes: list[dict[str, Any]]) -> str: ...
