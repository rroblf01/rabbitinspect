from rabbitinspect._core import analyze_code as _analyze_rs
from rabbitinspect._core import apply_fixes as _apply_fixes_rs

__all__ = ["analyze_code", "apply_fixes"]


def analyze_code(source: str) -> list[dict]:
    return _analyze_rs(source)


def apply_fixes(source: str, fixes: list[dict]) -> str:
    return _apply_fixes_rs(source, fixes)
