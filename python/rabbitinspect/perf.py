"""Sampling runtime profiler (F1 spike).

Thin Python layer over the Rust sampler in ``rabbitinspect._core``:

- :class:`Profiler` — start/stop context manager.
- :func:`aggregate` — turn raw samples into per-function self/total time.
- :func:`render_html` — self-contained HTML report (summary, top functions,
  memory-over-time chart, folded stacks for external flamegraph tools).
- :func:`profile_script` — launch a Python script under the profiler.

The sampler is statistical: timings are estimates derived from how often each
function was observed on the stack, not exact per-call measurements.
"""

from __future__ import annotations

import html
import json
import runpy
import sys
from collections import Counter
from dataclasses import dataclass, field

from rabbitinspect import _core


@dataclass
class FunctionStat:
    name: str
    file: str
    self_ms: float
    total_ms: float
    self_pct: float
    total_pct: float


@dataclass
class Span:
    method: str
    route: str
    status: int
    start_ms: float
    end_ms: float

    @property
    def duration_ms(self) -> float:
        return self.end_ms - self.start_ms


@dataclass
class EndpointStat:
    method: str
    route: str
    count: int
    p50_ms: float
    p95_ms: float
    p99_ms: float
    max_ms: float
    errors: int  # 5xx responses


@dataclass
class ProfileResult:
    duration_ms: float
    sample_count: int
    truncated: bool
    functions: list[FunctionStat]
    folded: list[tuple[str, int]]  # (root;...;leaf, count)
    rss: list[tuple[float, float]]  # (ms, bytes)
    spans: list[Span] = field(default_factory=list)
    endpoints: list[EndpointStat] = field(default_factory=list)
    raw: dict = field(default_factory=dict, repr=False)

    @property
    def peak_rss_bytes(self) -> float:
        return max((b for _, b in self.rss), default=0.0)

    def to_html(self) -> str:
        return render_html(self)


def _parse_frame(entry: str) -> tuple[str, str, int]:
    func, _, rest = entry.partition('\t')
    file, _, line = rest.partition('\t')
    try:
        lineno = int(line)
    except ValueError:
        lineno = 0
    return func, file, lineno


def aggregate(raw: dict) -> ProfileResult:
    """Reduce raw sampler output into per-function statistics."""
    frames: list[str] = raw['frames']
    stacks: list[list[int]] = raw['stacks']
    duration_ms: float = float(raw['duration_ms'])
    sample_count: int = int(raw['sample_count'])

    # Pre-parse the interned frame table once.
    parsed = [_parse_frame(f) for f in frames]

    self_counts: Counter[tuple[str, str]] = Counter()
    total_counts: Counter[tuple[str, str]] = Counter()
    folded_counts: Counter[str] = Counter()

    for stack in stacks:
        if not stack:
            continue
        # Leaf-first from the sampler; leaf == currently executing function.
        leaf_func, leaf_file, _ = parsed[stack[0]]
        self_counts[(leaf_func, leaf_file)] += 1

        seen: set[tuple[str, str]] = set()
        names_root_first: list[str] = []
        for fid in reversed(stack):
            func, file, _ = parsed[fid]
            key = (func, file)
            if key not in seen:
                seen.add(key)
            names_root_first.append(func)
        for key in seen:
            total_counts[key] += 1
        folded_counts[';'.join(names_root_first)] += 1

    denom = sample_count if sample_count else 1

    functions: list[FunctionStat] = []
    for key in total_counts:
        func, file = key
        sc = self_counts.get(key, 0)
        tc = total_counts[key]
        functions.append(
            FunctionStat(
                name=func,
                file=file,
                self_ms=sc / denom * duration_ms,
                total_ms=tc / denom * duration_ms,
                self_pct=sc / denom * 100.0,
                total_pct=tc / denom * 100.0,
            )
        )
    functions.sort(key=lambda f: f.self_ms, reverse=True)

    folded = sorted(folded_counts.items(), key=lambda kv: kv[1], reverse=True)
    rss = [(float(t), float(b)) for t, b in raw.get('rss', [])]

    spans = [
        Span(
            method=str(s.get('method', '')),
            route=str(s.get('route', '')),
            status=int(s.get('status', 0)),
            start_ms=float(s.get('start_ms', 0.0)),
            end_ms=float(s.get('end_ms', 0.0)),
        )
        for s in raw.get('spans', [])
    ]
    endpoints = _aggregate_endpoints(spans)

    return ProfileResult(
        duration_ms=duration_ms,
        sample_count=sample_count,
        truncated=bool(raw.get('truncated', False)),
        functions=functions,
        folded=folded,
        rss=rss,
        spans=spans,
        endpoints=endpoints,
        raw=raw,
    )


def _percentile(sorted_vals: list[float], pct: float) -> float:
    """Nearest-rank percentile of an already-sorted list."""
    if not sorted_vals:
        return 0.0
    if len(sorted_vals) == 1:
        return sorted_vals[0]
    rank = pct / 100.0 * (len(sorted_vals) - 1)
    lo = int(rank)
    hi = min(lo + 1, len(sorted_vals) - 1)
    frac = rank - lo
    return sorted_vals[lo] * (1 - frac) + sorted_vals[hi] * frac


def _aggregate_endpoints(spans: list[Span]) -> list[EndpointStat]:
    groups: dict[tuple[str, str], list[Span]] = {}
    for s in spans:
        groups.setdefault((s.method, s.route), []).append(s)

    stats: list[EndpointStat] = []
    for (method, route), group in groups.items():
        durations = sorted(s.duration_ms for s in group)
        stats.append(
            EndpointStat(
                method=method,
                route=route,
                count=len(group),
                p50_ms=_percentile(durations, 50),
                p95_ms=_percentile(durations, 95),
                p99_ms=_percentile(durations, 99),
                max_ms=durations[-1],
                errors=sum(1 for s in group if s.status >= 500),
            )
        )
    stats.sort(key=lambda e: e.count * e.p50_ms, reverse=True)
    return stats


class Profiler:
    """Context manager that samples the running interpreter.

    >>> with Profiler() as prof:
    ...     do_work()
    >>> prof.result.to_html()
    """

    def __init__(self, interval_ms: float = 5.0, max_depth: int = 256):
        self.interval_ms = interval_ms
        self.max_depth = max_depth
        self.result: ProfileResult | None = None

    def __enter__(self) -> 'Profiler':
        _core.perf_start(self.interval_ms, self.max_depth)
        return self

    def __exit__(self, *exc) -> None:
        raw = _core.perf_stop()
        self.result = aggregate(raw)


# ── HTML report ───────────────────────────────────────────────────────────


def _rss_svg(rss: list[tuple[float, float]], width: int = 900, height: int = 220) -> str:
    if len(rss) < 2:
        return '<p class="muted">Memory timeline unavailable on this platform.</p>'
    pad = 36
    xs = [t for t, _ in rss]
    ys = [b for _, b in rss]
    t0, t1 = min(xs), max(xs)
    y0, y1 = min(ys), max(ys)
    tspan = (t1 - t0) or 1.0
    yspan = (y1 - y0) or 1.0

    def px(t: float) -> float:
        return pad + (t - t0) / tspan * (width - 2 * pad)

    def py_(b: float) -> float:
        return height - pad - (b - y0) / yspan * (height - 2 * pad)

    pts = ' '.join(f'{px(t):.1f},{py_(b):.1f}' for t, b in rss)
    peak_mb = y1 / (1024 * 1024)
    base_mb = y0 / (1024 * 1024)
    return (
        f'<svg viewBox="0 0 {width} {height}" class="chart" role="img" aria-label="RSS over time">'
        f'<polyline fill="none" stroke="#2b8a3e" stroke-width="2" points="{pts}" />'
        f'<line x1="{pad}" y1="{height - pad}" x2="{width - pad}" y2="{height - pad}" stroke="#ccc"/>'
        f'<line x1="{pad}" y1="{pad}" x2="{pad}" y2="{height - pad}" stroke="#ccc"/>'
        f'<text x="{pad}" y="{pad - 8}" class="axis">{peak_mb:.1f} MB peak</text>'
        f'<text x="{pad}" y="{height - pad + 16}" class="axis">{base_mb:.1f} MB</text>'
        f'<text x="{width - pad}" y="{height - pad + 16}" class="axis" text-anchor="end">{t1:.0f} ms</text>'
        f'</svg>'
    )


def _func_rows(functions: list[FunctionStat], limit: int = 100) -> str:
    rows = []
    for f in functions[:limit]:
        bar = min(100.0, f.self_pct)
        rows.append(
            '<tr>'
            f'<td class="name">{html.escape(f.name)}</td>'
            f'<td class="file">{html.escape(f.file)}</td>'
            f'<td class="num">{f.self_ms:.1f}</td>'
            f'<td class="num">{f.total_ms:.1f}</td>'
            f'<td class="bar"><span style="width:{bar:.1f}%"></span>'
            f'<em>{f.self_pct:.1f}%</em></td>'
            '</tr>'
        )
    return '\n'.join(rows)


def _status_color(status: int) -> str:
    if status >= 500:
        return '#e03131'  # red
    if status >= 400:
        return '#f08c00'  # amber
    if status >= 200:
        return '#2b8a3e'  # green
    return '#868e96'  # grey / unknown


def _gantt_svg(spans: list[Span], duration_ms: float, width: int = 900, row_h: int = 14, max_rows: int = 200) -> str:
    if not spans:
        return ''
    pad = 4
    shown = spans[:max_rows]
    height = pad * 2 + row_h * len(shown)
    span_total = duration_ms or 1.0
    inner = width - 2 * pad
    bars = []
    for i, s in enumerate(shown):
        x = pad + max(0.0, s.start_ms) / span_total * inner
        w = max(1.0, (s.end_ms - s.start_ms) / span_total * inner)
        y = pad + i * row_h
        color = _status_color(s.status)
        label = html.escape(f'{s.method} {s.route} [{s.status}] {s.duration_ms:.0f}ms')
        bars.append(
            f'<rect x="{x:.1f}" y="{y}" width="{w:.1f}" height="{row_h - 2}" '
            f'fill="{color}" rx="1"><title>{label}</title></rect>'
        )
    extra = (
        f'<text x="{pad}" y="{height - 2}" class="axis">+{len(spans) - max_rows} more requests not shown</text>'
        if len(spans) > max_rows
        else ''
    )
    return (
        f'<svg viewBox="0 0 {width} {height}" class="chart" role="img" aria-label="Request timeline">'
        + ''.join(bars)
        + extra
        + '</svg>'
    )


def _endpoint_rows(endpoints: list[EndpointStat]) -> str:
    rows = []
    for e in endpoints:
        err = f'<span style="color:#e03131">{e.errors}</span>' if e.errors else '0'
        rows.append(
            '<tr>'
            f'<td class="name">{html.escape(e.method)} {html.escape(e.route)}</td>'
            f'<td class="num">{e.count}</td>'
            f'<td class="num">{e.p50_ms:.1f}</td>'
            f'<td class="num">{e.p95_ms:.1f}</td>'
            f'<td class="num">{e.p99_ms:.1f}</td>'
            f'<td class="num">{e.max_ms:.1f}</td>'
            f'<td class="num">{err}</td>'
            '</tr>'
        )
    return '\n'.join(rows)


def _requests_section(result: ProfileResult) -> str:
    if not result.spans:
        return ''
    return f"""
  <h2>Requests <span class="muted">({len(result.spans)} total, {len(result.endpoints)} endpoints)</span></h2>
  {_gantt_svg(result.spans, result.duration_ms)}
  <table>
    <thead><tr><th>Endpoint</th><th class="num">Count</th><th class="num">p50 ms</th><th class="num">p95 ms</th><th class="num">p99 ms</th><th class="num">max ms</th><th class="num">5xx</th></tr></thead>
    <tbody>
    {_endpoint_rows(result.endpoints)}
    </tbody>
  </table>
"""


def render_html(result: ProfileResult, title: str = 'rabbitinspect perf report') -> str:
    """Render a self-contained HTML report."""
    peak_mb = result.peak_rss_bytes / (1024 * 1024)
    folded_text = '\n'.join(f'{stack} {count}' for stack, count in result.folded)
    payload = json.dumps(
        {
            'duration_ms': result.duration_ms,
            'sample_count': result.sample_count,
            'functions': [vars(f) for f in result.functions],
            'rss': result.rss,
        }
    )
    # Escape so embedded data can't break out of the <script> block
    # (e.g. a function literally named "</script>").
    payload = payload.replace('<', '\\u003c').replace('>', '\\u003e').replace('&', '\\u0026')
    trunc = (
        '<p class="warn">⚠ Sample buffer was truncated; results are partial.</p>'
        if result.truncated
        else ''
    )
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{html.escape(title)}</title>
<style>
  body {{ font: 14px/1.5 system-ui, sans-serif; margin: 0; color: #222; background: #fafafa; }}
  header {{ background: #1f2937; color: #fff; padding: 16px 24px; }}
  header h1 {{ margin: 0; font-size: 18px; }}
  main {{ padding: 24px; max-width: 1100px; margin: 0 auto; }}
  .cards {{ display: flex; gap: 16px; flex-wrap: wrap; margin-bottom: 24px; }}
  .card {{ background: #fff; border: 1px solid #e5e7eb; border-radius: 8px; padding: 12px 16px; min-width: 140px; }}
  .card .v {{ font-size: 22px; font-weight: 600; }}
  .card .k {{ color: #6b7280; font-size: 12px; text-transform: uppercase; }}
  h2 {{ font-size: 15px; border-bottom: 2px solid #e5e7eb; padding-bottom: 6px; }}
  table {{ width: 100%; border-collapse: collapse; background: #fff; }}
  th, td {{ text-align: left; padding: 6px 8px; border-bottom: 1px solid #f0f0f0; }}
  th {{ font-size: 12px; color: #6b7280; text-transform: uppercase; }}
  td.num {{ text-align: right; font-variant-numeric: tabular-nums; }}
  td.file {{ color: #6b7280; font-size: 12px; }}
  td.bar {{ position: relative; width: 160px; }}
  td.bar span {{ display: inline-block; height: 12px; background: #3b82f6; border-radius: 2px; vertical-align: middle; }}
  td.bar em {{ font-style: normal; font-size: 11px; color: #6b7280; margin-left: 6px; }}
  .chart {{ width: 100%; max-width: 900px; background: #fff; border: 1px solid #e5e7eb; border-radius: 8px; }}
  .axis {{ font-size: 10px; fill: #9ca3af; }}
  .muted {{ color: #9ca3af; }}
  .warn {{ color: #b45309; }}
  details pre {{ background: #fff; border: 1px solid #e5e7eb; padding: 12px; overflow: auto; max-height: 320px; font-size: 12px; }}
</style>
</head>
<body>
<header><h1>{html.escape(title)}</h1></header>
<main>
  {trunc}
  <div class="cards">
    <div class="card"><div class="v">{result.duration_ms:.0f} ms</div><div class="k">Duration</div></div>
    <div class="card"><div class="v">{result.sample_count}</div><div class="k">Samples</div></div>
    <div class="card"><div class="v">{len(result.functions)}</div><div class="k">Functions</div></div>
    <div class="card"><div class="v">{peak_mb:.1f} MB</div><div class="k">Peak RSS</div></div>
  </div>

  <h2>Memory over time</h2>
  {_rss_svg(result.rss)}
  {_requests_section(result)}
  <h2>Top functions by self time</h2>
  <table>
    <thead><tr><th>Function</th><th>File</th><th class="num">Self ms</th><th class="num">Total ms</th><th>Self %</th></tr></thead>
    <tbody>
    {_func_rows(result.functions)}
    </tbody>
  </table>

  <h2>Folded stacks <span class="muted">(paste into a flamegraph tool)</span></h2>
  <details><summary>Show {len(result.folded)} collapsed stacks</summary>
  <pre>{html.escape(folded_text)}</pre>
  </details>

  <script type="application/json" id="rabbitinspect-perf-data">{payload}</script>
</main>
</body>
</html>
"""


# ── Launcher ────────────────────────────────────────────────────────────────


def profile_script(
    path: str,
    argv: list[str] | None = None,
    interval_ms: float = 5.0,
    max_depth: int = 256,
) -> ProfileResult:
    """Run ``path`` as ``__main__`` under the profiler and return the result."""
    saved_argv = sys.argv
    sys.argv = [path, *(argv or [])]
    prof = Profiler(interval_ms=interval_ms, max_depth=max_depth)
    try:
        with prof:
            try:
                runpy.run_path(path, run_name='__main__')
            except SystemExit:
                pass
    finally:
        sys.argv = saved_argv
    assert prof.result is not None
    return prof.result


def run_perf_cli(argv: list[str]) -> int:
    """Entry point for ``rabbitinspect perf ...``."""
    import argparse

    parser = argparse.ArgumentParser(
        prog='rabbitinspect perf',
        description='Sampling runtime profiler — launch a script and write an HTML report',
    )
    sub = parser.add_subparsers(dest='cmd', required=True)
    runp = sub.add_parser('run', help='Run a Python script under the profiler')
    runp.add_argument('--out', default='rabbitinspect-perf.html', help='HTML report output path')
    runp.add_argument('--interval', type=float, default=5.0, help='Sampling interval in ms')
    runp.add_argument('--max-depth', type=int, default=256, help='Maximum stack depth to walk')
    runp.add_argument('script', help='Python script to profile')
    runp.add_argument('script_args', nargs=argparse.REMAINDER, help='Arguments passed to the script')

    args = parser.parse_args(argv)
    if args.cmd == 'run':
        result = profile_script(
            args.script,
            args.script_args,
            interval_ms=args.interval,
            max_depth=args.max_depth,
        )
        with open(args.out, 'w', encoding='utf-8') as f:
            f.write(result.to_html())
        peak_mb = result.peak_rss_bytes / (1024 * 1024)
        print(
            f'Profiled {args.script}: {result.duration_ms:.0f} ms, '
            f'{result.sample_count} samples, {peak_mb:.1f} MB peak RSS',
            file=sys.stderr,
        )
        if result.functions:
            top = result.functions[0]
            print(f'Hottest: {top.name} ({top.self_pct:.1f}% self)', file=sys.stderr)
        print(f'Report written to {args.out}', file=sys.stderr)
        return 0
    return 1
