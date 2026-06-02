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

import ast
import html
import json
import os
import re
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
    off_cpu_ms: float = 0.0  # share of self time the thread was waiting (not on-CPU)
    line: int = 0  # the source line most often sampled for this function (0 = unknown)
    # per-line self time inside this function: (line, self_ms, samples), hottest first
    line_times: list = field(default_factory=list)


@dataclass
class MemAlloc:
    """Live allocation attributed to a function (from tracemalloc)."""

    function: str
    file: str
    line: int
    size_bytes: int
    count: int


@dataclass
class AsyncTaskInfo:
    """A snapshot of one asyncio task: its name, state, and coroutine stack."""

    name: str
    state: str  # 'pending' | 'done'
    stack: list[str]  # leaf-first "func\tfile\tline" entries


@dataclass
class Segment:
    """One time-ordered call interval reconstructed from consecutive samples.

    A sampling profiler never sees calls/returns directly; a run of consecutive
    samples that show the same function at the same stack depth (on one thread)
    is treated as a single continuous call. Two separate requests therefore yield
    two separate segments for the same function — the time-order view, not the
    aggregate.
    """

    func: str
    file: str
    line: int
    depth: int
    tid: int
    start_ms: float
    end_ms: float

    @property
    def duration_ms(self) -> float:
        return self.end_ms - self.start_ms


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
class Query:
    ts_ms: float
    sql: str
    normalized: str
    duration_ms: float
    origin: str = ''  # "file:line" of the app code that issued the query


@dataclass
class NPlusOne:
    method: str
    route: str
    normalized_sql: str
    max_count: int  # most repetitions seen within a single request
    requests_affected: int
    total_ms: float


@dataclass
class HotspotLint:
    function: str
    file: str
    self_ms: float
    findings: list[dict]  # {code, line, message}


@dataclass
class FunctionDelta:
    name: str
    file: str
    before_ms: float
    after_ms: float
    delta_ms: float  # after - before; positive == regression (slower)

    @property
    def delta_pct(self) -> float:
        if self.before_ms == 0:
            return 100.0 if self.after_ms > 0 else 0.0
        return (self.after_ms - self.before_ms) / self.before_ms * 100.0


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
    queries: list[Query] = field(default_factory=list)
    n_plus_one: list[NPlusOne] = field(default_factory=list)
    hotspot_lints: list[HotspotLint] = field(default_factory=list)
    on_cpu_ms: float = 0.0  # wall time sampled with at least one thread on-CPU
    off_cpu_ms: float = 0.0  # wall time sampled fully waiting (sleep / I/O / lock)
    mem_allocations: list['MemAlloc'] = field(default_factory=list)
    raw: dict = field(default_factory=dict, repr=False)

    @property
    def query_count(self) -> int:
        return len(self.queries)

    @property
    def query_total_ms(self) -> float:
        return sum(q.duration_ms for q in self.queries)

    @property
    def peak_rss_bytes(self) -> float:
        return max((b for _, b in self.rss), default=0.0)

    def to_html(self, app_root: str | None = None) -> str:
        return render_html(self, app_root=app_root)


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
    off_self_counts: Counter[tuple[str, str]] = Counter()
    total_counts: Counter[tuple[str, str]] = Counter()
    folded_counts: Counter[str] = Counter()
    line_counts: dict[tuple[str, str], Counter[int]] = {}
    leaf_line_counts: dict[tuple[str, str], Counter[int]] = {}

    # Per-sample OS thread state (remote attach only). 'R' == on-CPU; anything
    # else (S/D/…) == off-CPU (sleeping, blocked I/O, lock wait). Absent for the
    # in-process sampler, where all sampled threads are blocked on the GIL anyway.
    states: list[str] = raw.get('states', [])
    off_total = 0

    for i, stack in enumerate(stacks):
        if not stack:
            continue
        off_cpu = bool(states) and i < len(states) and states[i] != 'R'
        if off_cpu:
            off_total += 1
        # Leaf-first from the sampler; leaf == currently executing function.
        leaf_func, leaf_file, leaf_line = parsed[stack[0]]
        leaf_key = (leaf_func, leaf_file)
        self_counts[leaf_key] += 1
        if leaf_line:
            leaf_line_counts.setdefault(leaf_key, Counter())[leaf_line] += 1
        if off_cpu:
            off_self_counts[leaf_key] += 1

        seen: set[tuple[str, str]] = set()
        names_root_first: list[str] = []
        for fid in reversed(stack):
            func, file, line = parsed[fid]
            key = (func, file)
            if key not in seen:
                seen.add(key)
            if line:
                line_counts.setdefault(key, Counter())[line] += 1
            names_root_first.append(func)
        for key in seen:
            total_counts[key] += 1
        folded_counts[';'.join(names_root_first)] += 1

    denom = sample_count if sample_count else 1

    # Time is wall-clock per *tick*, not per per-thread stack. Each sampling tick
    # captures one stack per live thread, all stamped with the same timestamp; so
    # the number of distinct timestamps is the number of ticks. Weighting by ticks
    # (not by len(stacks)) means a function that ran for the whole window reads as
    # the full duration regardless of how many *other* threads were also sampled —
    # otherwise its time would be diluted by the concurrent-thread count. For the
    # single-threaded case ticks == sample_count, so this is unchanged.
    ts = raw.get('ts') or []
    num_ticks = len(set(ts)) if ts and len(ts) == len(stacks) else sample_count
    weight = duration_ms / num_ticks if num_ticks else 0.0

    functions: list[FunctionStat] = []
    for key in total_counts:
        func, file = key
        sc = self_counts.get(key, 0)
        tc = total_counts[key]
        self_ms = sc * weight
        total_ms = tc * weight
        lc = line_counts.get(key)
        rep_line = lc.most_common(1)[0][0] if lc else 0
        llc = leaf_line_counts.get(key)
        line_times = (
            sorted(((ln, cnt * weight, cnt) for ln, cnt in llc.items()), key=lambda t: -t[1])
            if llc
            else []
        )
        functions.append(
            FunctionStat(
                name=func,
                file=file,
                self_ms=self_ms,
                total_ms=total_ms,
                self_pct=(self_ms / duration_ms * 100.0) if duration_ms else 0.0,
                total_pct=(total_ms / duration_ms * 100.0) if duration_ms else 0.0,
                off_cpu_ms=off_self_counts.get(key, 0) * weight,
                line=rep_line,
                line_times=line_times,
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

    queries = [
        Query(
            ts_ms=float(q.get('ts_ms', 0.0)),
            sql=str(q.get('sql', '')),
            normalized=normalize_sql(str(q.get('sql', ''))),
            duration_ms=float(q.get('duration_ms', 0.0)),
            origin=str(q.get('origin', '')),
        )
        for q in raw.get('queries', [])
    ]
    n_plus_one = _detect_n_plus_one(spans, queries)

    off_cpu_ms = off_total / denom * duration_ms if states else 0.0
    on_cpu_ms = duration_ms - off_cpu_ms if states else 0.0

    return ProfileResult(
        duration_ms=duration_ms,
        sample_count=sample_count,
        truncated=bool(raw.get('truncated', False)),
        functions=functions,
        folded=folded,
        rss=rss,
        spans=spans,
        endpoints=endpoints,
        queries=queries,
        n_plus_one=n_plus_one,
        on_cpu_ms=on_cpu_ms,
        off_cpu_ms=off_cpu_ms,
        raw=raw,
    )


# ── SQL / N+1 ────────────────────────────────────────────────────────────────

_SQL_STR = re.compile(r"'[^']*'")
_SQL_NUM = re.compile(r'\b\d+\b')
_SQL_INLIST = re.compile(r'\(\s*\?(?:\s*,\s*\?)+\s*\)')
_SQL_WS = re.compile(r'\s+')


def normalize_sql(sql: str) -> str:
    """Collapse a query to its shape so duplicates group together.

    Replaces string/number literals with ``?`` and ``IN (?, ?, ?)`` with
    ``IN (?)`` so the same statement with different parameters is one key.
    """
    s = _SQL_STR.sub('?', sql)
    s = _SQL_NUM.sub('?', s)
    s = _SQL_INLIST.sub('(?)', s)
    s = _SQL_WS.sub(' ', s).strip()
    return s


def _detect_n_plus_one(spans: list[Span], queries: list[Query], threshold: int = 2) -> list[NPlusOne]:
    """Flag normalized queries repeated >= ``threshold`` times in one request."""
    if not queries:
        return []

    # Assign each query to the request span covering its timestamp.
    sorted_spans = sorted(spans, key=lambda s: s.start_ms)

    def request_of(ts: float) -> tuple[str, str] | None:
        for s in sorted_spans:
            if s.start_ms <= ts <= s.end_ms:
                return (s.method, s.route)
        return None

    # (request, normalized) -> list of per-request counts and durations.
    per_request: dict[tuple, dict[str, list[float]]] = {}
    NO_REQ = ('', '(no request)')
    for q in queries:
        req = request_of(q.ts_ms) or NO_REQ
        bucket = per_request.setdefault(req, {})
        bucket.setdefault(q.normalized, []).append(q.duration_ms)

    # Aggregate to endpoint level.
    agg: dict[tuple, dict] = {}
    for (method, route), buckets in per_request.items():
        for norm, durs in buckets.items():
            if len(durs) < threshold:
                continue
            key = (method, route, norm)
            entry = agg.setdefault(
                key, {'max_count': 0, 'requests_affected': 0, 'total_ms': 0.0}
            )
            entry['max_count'] = max(entry['max_count'], len(durs))
            entry['requests_affected'] += 1
            entry['total_ms'] += sum(durs)

    offenders = [
        NPlusOne(
            method=method,
            route=route,
            normalized_sql=norm,
            max_count=v['max_count'],
            requests_affected=v['requests_affected'],
            total_ms=v['total_ms'],
        )
        for (method, route, norm), v in agg.items()
    ]
    offenders.sort(key=lambda n: (n.max_count, n.total_ms), reverse=True)
    return offenders


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


def _func_for_line(file: str, line: int, cache: dict[str, dict[str, list[tuple[int, int]]]]) -> str:
    """Resolve which function a (file, line) lives in, via cached AST ranges."""
    ranges = cache.get(file)
    if ranges is None:
        try:
            with open(file, encoding='utf-8') as f:
                ranges = _function_ranges(f.read())
        except OSError:
            ranges = {}
        cache[file] = ranges
    best: tuple[int, str] | None = None  # (span, name); prefer the tightest match
    for name, spans in ranges.items():
        for start, end in spans:
            if start <= line <= end:
                span = end - start
                if best is None or span < best[0]:
                    best = (span, name)
    return best[1] if best else '<module>'


def _collect_tracemalloc(snapshot, top_n: int = 50) -> list[MemAlloc]:
    """Group a tracemalloc snapshot by source line and attribute it to functions."""
    cache: dict[str, dict[str, list[tuple[int, int]]]] = {}
    allocs: list[MemAlloc] = []
    for stat in snapshot.statistics('lineno'):
        frame = stat.traceback[0]
        file = frame.filename
        line = frame.lineno
        allocs.append(
            MemAlloc(
                function=_func_for_line(file, line, cache),
                file=file,
                line=line,
                size_bytes=stat.size,
                count=stat.count,
            )
        )
    allocs.sort(key=lambda a: a.size_bytes, reverse=True)
    return allocs[:top_n]


class Profiler:
    """Context manager that samples the running interpreter.

    >>> with Profiler() as prof:
    ...     do_work()
    >>> prof.result.to_html()

    Set ``trace_memory=True`` to additionally record per-function allocations
    via :mod:`tracemalloc` (adds overhead; off by default).
    """

    def __init__(self, interval_ms: float = 5.0, max_depth: int = 256, trace_memory: bool = False):
        self.interval_ms = interval_ms
        self.max_depth = max_depth
        self.trace_memory = trace_memory
        self.result: ProfileResult | None = None

    def __enter__(self) -> 'Profiler':
        self._started_tracemalloc = False
        if self.trace_memory:
            import tracemalloc

            if not tracemalloc.is_tracing():
                tracemalloc.start()
                self._started_tracemalloc = True
        try:
            _core.perf_start(self.interval_ms, self.max_depth)
        except Exception:
            # Don't leave tracemalloc running if the sampler refused to start
            # (e.g. another profiler is already active) — __exit__ won't run.
            # Only undo what *we* started, never someone else's tracing.
            if self._started_tracemalloc:
                import tracemalloc

                tracemalloc.stop()
            raise
        return self

    def __exit__(self, *exc) -> None:
        raw = _core.perf_stop()
        mem: list[MemAlloc] = []
        if self.trace_memory:
            import tracemalloc

            snapshot = tracemalloc.take_snapshot()
            if getattr(self, '_started_tracemalloc', True):
                tracemalloc.stop()
            mem = _collect_tracemalloc(snapshot)
        self.result = aggregate(raw)
        self.result.mem_allocations = mem


# ── hotspot ↔ lint cross-reference ───────────────────────────────────────────


def _function_ranges(source: str) -> dict[str, list[tuple[int, int]]]:
    """Map each function/method name to its [start, end] line ranges."""
    ranges: dict[str, list[tuple[int, int]]] = {}
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return ranges
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            start = node.lineno
            end = getattr(node, 'end_lineno', None) or start
            ranges.setdefault(node.name, []).append((start, end))
    return ranges


def analyze_hotspots(result: ProfileResult, top_n: int = 25) -> list[HotspotLint]:
    """Cross-reference the hottest functions against rabbitinspect's own lints.

    For each hot function whose source file is available, run the static
    analyzer and attach any findings located inside that function. This is the
    toolkit's differentiator: it points the static rules straight at the code
    that actually dominates runtime.
    """
    from rabbitinspect import analyze_code

    cache: dict[str, tuple[list[dict], dict[str, list[tuple[int, int]]]] | None] = {}
    out: list[HotspotLint] = []

    for fn in result.functions[:top_n]:
        path = fn.file
        if not path.endswith('.py') or not os.path.exists(path):
            continue
        if path not in cache:
            try:
                with open(path, encoding='utf-8') as f:
                    src = f.read()
            except OSError:
                cache[path] = None
                continue
            cache[path] = (analyze_code(src), _function_ranges(src))
        cached = cache[path]
        if cached is None:
            continue
        findings, ranges = cached
        spans = ranges.get(fn.name)
        if not spans:
            continue
        matched = [
            {'code': f['code'], 'line': f['line'], 'message': f['message']}
            for f in findings
            if any(lo <= f['line'] <= hi for lo, hi in spans)
        ]
        if matched:
            out.append(HotspotLint(function=fn.name, file=path, self_ms=fn.self_ms, findings=matched))

    result.hotspot_lints = out
    return out


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


def _source_lines(file: str, cache: dict[str, list[str] | None]) -> list[str] | None:
    """Read & cache a source file's lines (for per-line breakdowns). None if unreadable."""
    if file in cache:
        return cache[file]
    lines: list[str] | None
    try:
        with open(file, encoding='utf-8', errors='replace') as f:
            lines = f.read().splitlines()
    except OSError:
        lines = None
    cache[file] = lines
    return lines


def _line_detail(f: FunctionStat, cache: dict[str, list[str] | None]) -> str:
    """A per-line self-time breakdown inside one function (with source text)."""
    if not f.line_times:
        return ''
    src = _source_lines(f.file, cache)
    total = f.self_ms or 1.0
    rows = []
    for ln, ms, _cnt in f.line_times:
        pct = min(100.0, ms / total * 100.0)
        code = ''
        if src and 1 <= ln <= len(src):
            code = html.escape(src[ln - 1].rstrip()[:200])
        rows.append(
            '<tr>'
            f'<td class="num">{ln}</td>'
            f'<td class="num">{ms:.1f}</td>'
            f'<td class="linebar"><span style="width:{pct:.1f}%"></span></td>'
            f'<td><code>{code}</code></td>'
            '</tr>'
        )
    return (
        '<table class="lines"><thead><tr>'
        '<th class="num">Line</th><th class="num">Self ms</th><th></th><th>Code</th>'
        f'</tr></thead><tbody>{"".join(rows)}</tbody></table>'
    )


def _func_rows(functions: list[FunctionStat], limit: int = 100) -> str:
    rows = []
    src_cache: dict[str, list[str] | None] = {}
    for f in functions[:limit]:
        bar = min(100.0, f.self_pct)
        has_detail = bool(f.line_times)
        marker = '<span class="tw">▸</span> ' if has_detail else ''
        rows.append(
            f'<tr class="{"fn" if has_detail else ""}">'
            f'<td class="name">{marker}{html.escape(f.name)}</td>'
            f'<td class="file">{html.escape(f.file)}{f":{f.line}" if f.line else ""}</td>'
            f'<td class="num">{f.self_ms:.1f}</td>'
            f'<td class="num">{f.total_ms:.1f}</td>'
            f'<td class="num">{f.off_cpu_ms:.1f}</td>'
            f'<td class="bar"><span style="width:{bar:.1f}%"></span>'
            f'<em>{f.self_pct:.1f}%</em></td>'
            '</tr>'
        )
        if has_detail:
            rows.append(
                f'<tr class="fndetail" style="display:none"><td colspan="6">{_line_detail(f, src_cache)}</td></tr>'
            )
    return '\n'.join(rows)


def _is_app_frame(file: str, root: str) -> bool:
    """True if ``file`` is the user's own code under ``root`` (not stdlib/deps).

    Excludes synthetic frames (``<frozen ...>``), anything outside ``root``, and
    third-party installs (site-packages / venv) so the report can surface the
    application's own functions instead of framework/idle noise.
    """
    if not file or file.startswith('<'):
        return False
    # Normalize separators so the check is OS-independent (Windows uses '\').
    f = file.replace('\\', '/')
    r = root.replace('\\', '/')
    if not f.startswith(r):
        return False
    parts = f.split('/')
    return not any(p in ('site-packages', 'dist-packages', '.venv', 'venv') for p in parts)


def _app_functions_section(functions: list[FunctionStat], app_root: str | None) -> str:
    if not app_root:
        return ''
    root = os.path.realpath(app_root)
    app = [f for f in functions if _is_app_frame(f.file, root)]
    if not app:
        return (
            '\n  <h2>Application functions <span class="muted">'
            f'(under {html.escape(root)})</span></h2>'
            '\n  <p class="muted">No samples landed in your own code during this run '
            '(the time was spent in the framework / standard library / waiting).</p>\n'
        )
    return f"""
  <h2>Top application functions <span class="muted">(your code under {html.escape(root)}; excludes stdlib &amp; dependencies)</span></h2>
  <table>
    <thead><tr><th>Function</th><th>File</th><th class="num">Self ms</th><th class="num">Total ms</th><th class="num">Wait ms</th><th>Self %</th></tr></thead>
    <tbody>
    {_func_rows(app)}
    </tbody>
  </table>
"""


def _mem_section(allocs: list[MemAlloc], limit: int = 25) -> str:
    if not allocs:
        return ''
    rows = []
    for a in allocs[:limit]:
        kb = a.size_bytes / 1024.0
        rows.append(
            '<tr>'
            f'<td class="name">{html.escape(a.function)}</td>'
            f'<td class="file">{html.escape(a.file)}:{a.line}</td>'
            f'<td class="num">{kb:.1f}</td>'
            f'<td class="num">{a.count}</td>'
            '</tr>'
        )
    return f"""
  <h2>Top allocations by size <span class="muted">(tracemalloc; live at stop)</span></h2>
  <table>
    <thead><tr><th>Function</th><th>Site</th><th class="num">KB</th><th class="num">Blocks</th></tr></thead>
    <tbody>
    {chr(10).join(rows)}
    </tbody>
  </table>
"""


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


def _database_section(result: ProfileResult) -> str:
    if not result.queries:
        return ''
    slowest = sorted(result.queries, key=lambda q: q.duration_ms, reverse=True)[:10]
    slow_rows = '\n'.join(
        '<tr>'
        f'<td class="num">{q.duration_ms:.1f}</td>'
        f'<td class="file">{html.escape(q.origin) if q.origin else "—"}</td>'
        f'<td class="name"><code>{html.escape(q.sql[:200])}</code></td>'
        '</tr>'
        for q in slowest
    )
    npo_rows = '\n'.join(
        '<tr>'
        f'<td class="name">{html.escape(n.method)} {html.escape(n.route)}</td>'
        f'<td class="num">{n.max_count}</td>'
        f'<td class="num">{n.requests_affected}</td>'
        f'<td class="num">{n.total_ms:.1f}</td>'
        f'<td class="name"><code>{html.escape(n.normalized_sql[:200])}</code></td>'
        '</tr>'
        for n in result.n_plus_one
    )
    npo_block = (
        f"""
  <h3>Possible N+1 queries <span class="muted">({len(result.n_plus_one)})</span></h3>
  <table>
    <thead><tr><th>Endpoint</th><th class="num">Max/req</th><th class="num">Requests</th><th class="num">Total ms</th><th>Query shape</th></tr></thead>
    <tbody>{npo_rows}</tbody>
  </table>
"""
        if result.n_plus_one
        else '<p class="muted">No repeated-query (N+1) patterns detected.</p>'
    )
    return f"""
  <h2>Database <span class="muted">({result.query_count} queries, {result.query_total_ms:.1f} ms total)</span></h2>
  {npo_block}
  <h3>Slowest queries</h3>
  <table>
    <thead><tr><th class="num">ms</th><th>Origin</th><th>Query</th></tr></thead>
    <tbody>{slow_rows}</tbody>
  </table>
"""


def _hotspot_lints_section(result: ProfileResult, app_root: str | None = None) -> str:
    hotspots = result.hotspot_lints
    if app_root:
        # Linting stdlib / dependencies is noise — restrict to the user's code.
        root = os.path.realpath(app_root)
        hotspots = [h for h in hotspots if _is_app_frame(h.file, root)]
    if not hotspots:
        return ''
    blocks = []
    for h in hotspots:
        items = '\n'.join(
            f'<li><strong>{html.escape(f["code"])}</strong> '
            f'<span class="muted">line {f["line"]}</span> — {html.escape(f["message"])}</li>'
            for f in h.findings
        )
        blocks.append(
            f'<div class="hotspot"><div class="hot-head">'
            f'<strong>{html.escape(h.function)}</strong> '
            f'<span class="muted">{html.escape(h.file)} · {h.self_ms:.1f} ms self</span></div>'
            f'<ul>{items}</ul></div>'
        )
    return f"""
  <h2>Hotspots with lint findings <span class="muted">(hot code that also trips a static rule)</span></h2>
  {''.join(blocks)}
"""


def _zoom_script(css_class: str) -> str:
    """Click-to-zoom JS for any SVG of ``css_class`` whose <g> carry data-f0/f1
    (normalized [0,1] x-range). Lives in HTML context (currentScript is null for
    SVG <script>); reads width from the SVG viewBox. Same logic for the
    aggregate flamegraph and the time-order flame chart."""
    return (
        '<script>'
        'document.querySelectorAll("svg.' + css_class + '").forEach(function(s){'
        'var W=s.viewBox.baseVal.width;'
        'var gs=[].slice.call(s.querySelectorAll("g[data-f0]"));'
        'function zoom(a,b){var sp=b-a;if(sp<=0)return;gs.forEach(function(g){'
        'var f0=+g.dataset.f0,f1=+g.dataset.f1,n0=(f0-a)/sp,n1=(f1-a)/sp;'
        'if(n1<=0.0001||n0>=0.9999){g.style.display="none";return;}g.style.display="";'
        'var r=g.querySelector("rect"),t=g.querySelector("text");'
        'var x=Math.max(0,n0)*W,w=(Math.min(1,n1)-Math.max(0,n0))*W;'
        'r.setAttribute("x",x.toFixed(1));r.setAttribute("width",Math.max(w-1,0.5).toFixed(1));'
        'if(t){t.setAttribute("x",(x+3).toFixed(1));t.style.display=w>28?"":"none";}});}'
        'gs.forEach(function(g){g.style.cursor="pointer";g.addEventListener("click",function(e){'
        'e.stopPropagation();zoom(+g.dataset.f0,+g.dataset.f1);});});'
        's.addEventListener("click",function(){zoom(0,1);});'
        '});</script>'
    )


def _flame_color(name: str) -> str:
    """Stable warm flamegraph color (orange-yellow band) from a frame name."""
    h = 0
    for ch in name:
        h = (h * 31 + ord(ch)) & 0xFFFFFFFF
    hue = 18 + h % 42  # 18..59 → red-orange to yellow
    light = 50 + (h >> 8) % 12  # 50..61
    return f'hsl({hue},85%,{light}%)'


def _flamegraph_svg(result: ProfileResult, width: int = 1100, row_h: int = 18, with_script: bool = True) -> str:
    """Render an inline icicle flamegraph (root at top) from folded stacks.

    Each frame's width is proportional to its sample count; children sit below
    their parent. Self-contained SVG — no JS required to view the shape.
    """
    if not result.folded:
        return '<p class="muted">No stacks captured.</p>'

    # Build a call tree of frames; each node tracks its cumulative sample count.
    class _Node:
        __slots__ = ('count', 'children')

        def __init__(self) -> None:
            self.count = 0
            self.children: dict[str, _Node] = {}

    root_children: dict[str, _Node] = {}
    total = 0
    for path, count in result.folded:
        total += count
        node = root_children
        for name in path.split(';'):
            child = node.get(name)
            if child is None:
                child = _Node()
                node[name] = child
            child.count += count
            node = child.children
    if total == 0:
        return '<p class="muted">No stacks captured.</p>'

    pad = 2
    scale = (width - 2 * pad) / total
    rects: list[str] = []
    max_depth = 0

    def emit(children: dict[str, _Node], offset: int, depth: int) -> None:
        """offset is the cumulative sample index where this group starts."""
        nonlocal max_depth
        max_depth = max(max_depth, depth)
        x0 = offset
        # Stable left-to-right order: heaviest first, then by name.
        for name, child in sorted(children.items(), key=lambda kv: (-kv[1].count, kv[0])):
            count = child.count
            x = pad + x0 * scale
            w = count * scale
            y = depth * row_h
            ms = count / total * result.duration_ms
            pct = count / total * 100
            label = name.split('\t')[0]
            title = html.escape(f'{label} — {count} samples ({pct:.1f}%, {ms:.0f} ms)')
            # fractions of the full width — used by the zoom script to rescale.
            f0 = x0 / total
            f1 = (x0 + count) / total
            text = ''
            if w > 28:
                shown = label if len(label) * 6.5 < w else label[: max(1, int(w / 6.5))] + '…'
                text = (
                    f'<text x="{x + 3:.1f}" y="{y + row_h - 5}" '
                    f'font-size="11" fill="#1a1a1a" pointer-events="none">{html.escape(shown)}</text>'
                )
            rects.append(
                f'<g data-f0="{f0:.6f}" data-f1="{f1:.6f}" data-y="{y}">'
                f'<title>{title}</title>'
                f'<rect x="{x:.1f}" y="{y}" width="{max(w - 1, 0.5):.1f}" height="{row_h - 1}" '
                f'rx="1.5" fill="{_flame_color(label)}" stroke="#fff" stroke-width="0.5"/>'
                f'{text}</g>'
            )
            emit(child.children, x0, depth + 1)
            x0 += count

    emit(root_children, 0, 0)
    height = (max_depth + 1) * row_h + pad
    svg = (
        f'<svg viewBox="0 0 {width} {height}" class="chart flame" role="img" '
        f'aria-label="Flamegraph" preserveAspectRatio="xMidYMin meet">'
        f'{"".join(rects)}</svg>'
    )
    return svg + _zoom_script('flame') if with_script else svg


def _build_segments(raw: dict) -> list[Segment]:
    """Reconstruct time-ordered call segments from raw samples.

    Per thread (by ``tid``), a run of consecutive samples showing the same
    function at the same depth becomes one segment. A function called twice
    produces two segments. Identity is (func, file) — line is kept for display
    but ignored for merging, so a function running across many lines stays one
    segment instead of fragmenting per source line.
    """
    frames = raw.get('frames') or []
    stacks = raw.get('stacks') or []
    ts = raw.get('ts') or []
    tids = raw.get('tids') or []
    if not stacks or len(ts) != len(stacks):
        return []
    parsed = [_parse_frame(f) for f in frames]

    groups: dict[int, list[int]] = {}
    for i in range(len(stacks)):
        groups.setdefault(tids[i] if i < len(tids) else 0, []).append(i)

    segments: list[Segment] = []
    for tid, idxs in groups.items():
        idxs.sort(key=lambda i: ts[i])
        tss = [ts[i] for i in idxs]
        gaps = sorted(b - a for a, b in zip(tss, tss[1:]) if b > a)
        interval = gaps[len(gaps) // 2] if gaps else 1.0
        open_segs: list[dict] = []  # root-first, currently-open frames
        for i in idxs:
            t = ts[i]
            keys = [parsed[fid] for fid in reversed(stacks[i])]  # (func, file, line) root-first
            common = 0
            while (
                common < len(open_segs)
                and common < len(keys)
                and open_segs[common]['func'] == keys[common][0]
                and open_segs[common]['file'] == keys[common][1]
            ):
                common += 1
            while len(open_segs) > common:  # close frames that ended
                seg = open_segs.pop()
                segments.append(
                    Segment(seg['func'], seg['file'], seg['line'], len(open_segs), tid, seg['start'], t)
                )
            for d in range(common, len(keys)):
                func, file, line = keys[d]
                open_segs.append({'func': func, 'file': file, 'line': line, 'start': t})
        end = (tss[-1] + interval) if tss else 0.0
        while open_segs:  # flush still-open frames at the end
            seg = open_segs.pop()
            segments.append(
                Segment(seg['func'], seg['file'], seg['line'], len(open_segs), tid, seg['start'], end)
            )
    return segments


def _flamechart_svg(segments: list[Segment], duration_ms: float, width: int = 1100, row_h: int = 16) -> str:
    """Time-order flame chart: x = wall-clock time, one box per call.

    Threads are stacked in their own vertical bands. Hovering a box shows that
    call's duration. Click to zoom a time range, click the background to reset.
    """
    if not segments or duration_ms <= 0:
        return '<p class="muted">Time-order view needs timestamped samples.</p>'

    max_boxes = 6000
    truncated = len(segments) > max_boxes
    segs = sorted(segments, key=lambda s: s.duration_ms, reverse=True)[:max_boxes] if truncated else segments

    by_tid: dict[int, list[Segment]] = {}
    for s in segs:
        by_tid.setdefault(s.tid, []).append(s)

    pad = 2
    drawable = width - 2 * pad
    scale = drawable / duration_ms
    label_h = 14
    boxes: list[str] = []
    y = 0
    for tid in sorted(by_tid):
        tsegs = by_tid[tid]
        max_d = max(s.depth for s in tsegs)
        boxes.append(f'<text x="{pad}" y="{y + 11}" font-size="11" fill="#374151">thread {tid}</text>')
        band_top = y + label_h
        for s in tsegs:
            x = pad + s.start_ms * scale
            w = max(s.duration_ms * scale, 0.4)
            by = band_top + s.depth * row_h
            loc = f'{s.file}:{s.line}' if s.line else s.file
            title = html.escape(f'{s.func}  —  {s.duration_ms:.1f} ms\n{loc}')
            text = ''
            if w > 30:
                shown = s.func if len(s.func) * 6.5 < w else s.func[: max(1, int(w / 6.5))] + '…'
                text = (
                    f'<text x="{x + 3:.1f}" y="{by + row_h - 4}" font-size="10" '
                    f'fill="#1a1a1a" pointer-events="none">{html.escape(shown)}</text>'
                )
            boxes.append(
                f'<g data-f0="{s.start_ms / duration_ms:.6f}" data-f1="{s.end_ms / duration_ms:.6f}">'
                f'<title>{title}</title>'
                f'<rect x="{x:.1f}" y="{by}" width="{w:.1f}" height="{row_h - 1}" rx="1" '
                f'fill="{_flame_color(s.func)}" stroke="#fff" stroke-width="0.4"/>{text}</g>'
            )
        y = band_top + (max_d + 1) * row_h + 6
    height = y + pad
    note = f'<p class="muted">Showing the {max_boxes} longest calls.</p>' if truncated else ''
    svg = (
        f'<svg viewBox="0 0 {width} {height}" class="chart flamechart" role="img" '
        f'aria-label="Flame chart (time order)" preserveAspectRatio="xMidYMin meet">'
        f'{"".join(boxes)}</svg>'
    )
    return note + svg + _zoom_script('flamechart')


def _call_timings_section(segments: list[Segment], limit: int = 40) -> str:
    if not segments:
        return ''
    groups: dict[tuple[str, str], list[float]] = {}
    for s in segments:
        groups.setdefault((s.func, s.file), []).append(s.duration_ms)
    items = sorted(groups.items(), key=lambda kv: sum(kv[1]), reverse=True)
    rows = []
    for (func, file), durs in items[:limit]:
        n = len(durs)
        total = sum(durs)
        rows.append(
            '<tr>'
            f'<td class="name">{html.escape(func)}</td>'
            f'<td class="file">{html.escape(file)}</td>'
            f'<td class="num">{n}</td>'
            f'<td class="num">{total:.1f}</td>'
            f'<td class="num">{total / n:.1f}</td>'
            f'<td class="num">{max(durs):.1f}</td>'
            '</tr>'
        )
    return f"""
  <h2>Call timings <span class="muted">(time-order: every call counted separately)</span></h2>
  <table>
    <thead><tr><th>Function</th><th>File</th><th class="num">Calls</th><th class="num">Total ms</th><th class="num">Avg ms</th><th class="num">Max ms</th></tr></thead>
    <tbody>
    {chr(10).join(rows)}
    </tbody>
  </table>
"""


def _subprofile_for_windows(raw: dict, windows: list[tuple[float, float]]) -> ProfileResult:
    """A sub-profile of only the samples whose timestamp falls inside any window.

    Used to build a per-endpoint flamegraph: keep the samples taken while a
    request to that endpoint was in flight.
    """
    ts = raw.get('ts') or []
    stacks = raw.get('stacks') or []
    tids = raw.get('tids') or []
    states = raw.get('states') or []
    keep = [i for i, t in enumerate(ts) if any(a <= t <= b for a, b in windows)]
    sub = {
        'frames': raw.get('frames') or [],
        'stacks': [stacks[i] for i in keep],
        'ts': [ts[i] for i in keep],
        'tids': [tids[i] if i < len(tids) else 0 for i in keep],
        'states': [states[i] if i < len(states) else 'R' for i in keep],
        'rss': [],
        'spans': [],
        'queries': [],
        'duration_ms': sum(b - a for a, b in windows),
        'sample_count': len(keep),
        'truncated': False,
    }
    return aggregate(sub)


def _endpoint_flamegraphs(result: ProfileResult, top_n: int = 5) -> str:
    """A flamegraph per endpoint: where each route spends its time."""
    if not result.endpoints or not (result.raw.get('ts')):
        return ''
    blocks = []
    for ep in result.endpoints[:top_n]:
        windows = [
            (s.start_ms, s.end_ms)
            for s in result.spans
            if s.method == ep.method and s.route == ep.route and s.end_ms > s.start_ms
        ]
        if not windows:
            continue
        sub = _subprofile_for_windows(result.raw, windows)
        if not sub.folded:
            continue
        blocks.append(
            f'<h3>{html.escape(ep.method)} {html.escape(ep.route)} '
            f'<span class="muted">({ep.count} req, p95 {ep.p95_ms:.0f} ms)</span></h3>'
            + _flamegraph_svg(sub, with_script=False)
        )
    if not blocks:
        return ''
    return (
        '\n  <h2>Per-endpoint flamegraphs <span class="muted">'
        '(time spent while each route was serving)</span></h2>\n  '
        + '\n  '.join(blocks)
    )


def render_html(
    result: ProfileResult,
    title: str = 'rabbitinspect perf report',
    app_root: str | None = None,
) -> str:
    """Render a self-contained HTML report.

    If ``app_root`` is given, an extra "Top application functions" section lists
    only the user's own code under that path (stdlib and dependencies excluded),
    cutting through framework / idle-thread noise.
    """
    peak_mb = result.peak_rss_bytes / (1024 * 1024)
    cpu_card = ''
    if result.on_cpu_ms or result.off_cpu_ms:
        total = result.on_cpu_ms + result.off_cpu_ms or 1.0
        on_pct = result.on_cpu_ms / total * 100.0
        cpu_card = (
            f'<div class="card"><div class="v">{on_pct:.0f}%</div>'
            f'<div class="k">On-CPU</div></div>'
        )
    segments = _build_segments(result.raw)
    flamechart = ''
    if segments:
        flamechart = (
            '\n  <h2>Flame chart <span class="muted">(time order — each call shown separately; hover for duration, click to zoom)</span></h2>\n  '
            + _flamechart_svg(segments, result.duration_ms)
            + '\n'
            + _call_timings_section(segments)
        )
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
  th.num {{ text-align: right; }}
  td.file {{ color: #6b7280; font-size: 12px; }}
  td.bar {{ position: relative; width: 160px; }}
  td.bar span {{ display: inline-block; height: 12px; background: #3b82f6; border-radius: 2px; vertical-align: middle; }}
  td.bar em {{ font-style: normal; font-size: 11px; color: #6b7280; margin-left: 6px; }}
  tr.fn {{ cursor: pointer; }}
  tr.fn:hover {{ background: #f3f4f6; }}
  tr.fn td.name {{ font-weight: 500; }}
  tr.fndetail > td {{ background: #f9fafb; padding: 4px 8px 10px 24px; }}
  table.lines {{ width: auto; min-width: 60%; background: transparent; }}
  table.lines td, table.lines th {{ border-bottom: 1px solid #eee; padding: 2px 8px; }}
  table.lines td.linebar {{ width: 120px; }}
  table.lines td.linebar span {{ display: inline-block; height: 9px; background: #f08c00; border-radius: 2px; }}
  table.lines code {{ white-space: pre; }}
  .chart {{ width: 100%; max-width: 900px; background: #fff; border: 1px solid #e5e7eb; border-radius: 8px; }}
  .flame {{ max-width: 1100px; }}
  .flame rect {{ cursor: default; }}
  .flame g:hover rect {{ stroke: #1f2937; stroke-width: 1; }}
  .flamechart {{ max-width: 1100px; }}
  .flamechart g:hover rect {{ stroke: #1f2937; stroke-width: 1; }}
  .axis {{ font-size: 10px; fill: #9ca3af; }}
  .muted {{ color: #9ca3af; }}
  .warn {{ color: #b45309; }}
  h3 {{ font-size: 13px; color: #374151; margin-top: 16px; }}
  code {{ font-family: ui-monospace, monospace; font-size: 12px; }}
  .hotspot {{ background: #fff; border: 1px solid #e5e7eb; border-left: 3px solid #f08c00; border-radius: 6px; padding: 8px 12px; margin-bottom: 8px; }}
  .hotspot ul {{ margin: 6px 0 0; padding-left: 18px; }}
  .hot-head {{ margin-bottom: 2px; }}
  details pre {{ background: #fff; border: 1px solid #e5e7eb; padding: 12px; overflow: auto; max-height: 320px; font-size: 12px; }}
  .toolbar {{ display: flex; gap: 8px; align-items: center; margin-bottom: 16px; }}
  #fsearch {{ flex: 1; max-width: 360px; padding: 6px 10px; border: 1px solid #d1d5db; border-radius: 6px; font: inherit; }}
  .tbtn {{ padding: 6px 10px; border: 1px solid #d1d5db; border-radius: 6px; background: #fff; cursor: pointer; font: inherit; }}
  body.dark {{ background: #0f172a; color: #e5e7eb; }}
  body.dark header {{ background: #020617; }}
  body.dark .card, body.dark table, body.dark .chart, body.dark .hotspot, body.dark details pre, body.dark #fsearch, body.dark .tbtn {{ background: #1e293b; border-color: #334155; color: #e5e7eb; }}
  body.dark td, body.dark th {{ border-color: #334155; }}
  body.dark tr.fndetail > td {{ background: #16233b; }}
  body.dark .muted, body.dark td.file, body.dark .card .k, body.dark th {{ color: #94a3b8; }}
</style>
</head>
<body>
<header><h1>{html.escape(title)}</h1></header>
<main>
  {trunc}
  <div class="toolbar">
    <input id="fsearch" placeholder="filter functions…" oninput="rabFilter(this.value)">
    <button class="tbtn" onclick="document.body.classList.toggle('dark')">🌓 theme</button>
    <button class="tbtn" onclick="rabExportCsv()">⬇ functions.csv</button>
  </div>
  <div class="cards">
    <div class="card"><div class="v">{result.duration_ms:.0f} ms</div><div class="k">Duration</div></div>
    <div class="card"><div class="v">{result.sample_count}</div><div class="k">Samples</div></div>
    <div class="card"><div class="v">{len(result.functions)}</div><div class="k">Functions</div></div>
    <div class="card"><div class="v">{peak_mb:.1f} MB</div><div class="k">Peak RSS</div></div>
    {cpu_card}
  </div>

  <h2>Memory over time</h2>
  {_rss_svg(result.rss)}
  {_mem_section(result.mem_allocations)}
  {_requests_section(result)}
  {_endpoint_flamegraphs(result)}
  {_database_section(result)}
  {_hotspot_lints_section(result, app_root)}
  <h2>Flamegraph <span class="muted">(width = share of samples; click to zoom, click background to reset)</span></h2>
  {_flamegraph_svg(result)}
  {flamechart}
  {_app_functions_section(result.functions, app_root)}
  <h2>All functions by self time <span class="muted">(includes stdlib, dependencies &amp; idle threads)</span></h2>
  <table>
    <thead><tr><th>Function</th><th>File</th><th class="num">Self ms</th><th class="num">Total ms</th><th class="num">Wait ms</th><th>Self %</th></tr></thead>
    <tbody>
    {_func_rows(result.functions)}
    </tbody>
  </table>

  <h2>Folded stacks <span class="muted">(paste into a flamegraph tool)</span></h2>
  <details><summary>Show {len(result.folded)} collapsed stacks</summary>
  <pre>{html.escape(folded_text)}</pre>
  </details>

  <script type="application/json" id="rabbitinspect-perf-data">{payload}</script>
  <script>document.querySelectorAll("tr.fn").forEach(function(r){{r.addEventListener("click",function(){{var d=r.nextElementSibling;if(d&&d.classList.contains("fndetail")){{var open=d.style.display==="none";d.style.display=open?"table-row":"none";var tw=r.querySelector(".tw");if(tw)tw.textContent=open?"▾":"▸";}}}});}});
  function rabFilter(q){{q=q.toLowerCase();document.querySelectorAll("table tr").forEach(function(r){{var n=r.querySelector("td.name");if(!n)return;var hit=!q||r.textContent.toLowerCase().indexOf(q)>=0;r.style.display=hit?"":"none";var d=r.nextElementSibling;if(d&&d.classList.contains("fndetail"))d.style.display="none";}});}}
  function rabExportCsv(){{var el=document.getElementById("rabbitinspect-perf-data");if(!el)return;var fns=JSON.parse(el.textContent).functions||[];var rows=["function,file,line,self_ms,total_ms,off_cpu_ms,self_pct"];fns.forEach(function(f){{rows.push([f.name,f.file,f.line,f.self_ms,f.total_ms,f.off_cpu_ms,f.self_pct].map(function(x){{return '"'+String(x).replace(/"/g,'""')+'"';}}).join(","));}});var blob=new Blob([rows.join("\\n")],{{type:"text/csv"}});var a=document.createElement("a");a.href=URL.createObjectURL(blob);a.download="functions.csv";a.click();}}</script>
</main>
</body>
</html>
"""


# ── speedscope export ─────────────────────────────────────────────────────


def to_speedscope(result: ProfileResult, name: str = 'rabbitinspect') -> dict:
    """Export the profile as a speedscope file (https://speedscope.app).

    Lets users open an interactive flamegraph / time-order view in a mature,
    battle-tested viewer instead of relying only on the built-in HTML.
    """
    frames_raw: list[str] = result.raw.get('frames', [])
    stacks: list[list[int]] = result.raw.get('stacks', [])

    shared_frames = []
    for entry in frames_raw:
        func, _, rest = entry.partition('\t')
        file, _, line = rest.partition('\t')
        frame: dict = {'name': func}
        if file:
            frame['file'] = file
        try:
            frame['line'] = int(line)
        except ValueError:
            pass
        shared_frames.append(frame)

    n = len(stacks)
    weight = (result.duration_ms / n) if n else 0.0
    # speedscope wants each sample as root-first frame indices; our stacks are leaf-first.
    samples = [list(reversed(stack)) for stack in stacks]
    weights = [weight] * n

    return {
        '$schema': 'https://www.speedscope.app/file-format-schema.json',
        'name': name,
        'exporter': 'rabbitinspect',
        'shared': {'frames': shared_frames},
        'profiles': [
            {
                'type': 'sampled',
                'name': name,
                'unit': 'milliseconds',
                'startValue': 0,
                'endValue': result.duration_ms,
                'samples': samples,
                'weights': weights,
            }
        ],
    }


# ── profile diff (before / after) ─────────────────────────────────────────


def _diff_flamegraph_svg(before: ProfileResult, after: ProfileResult, width: int = 1100, row_h: int = 18) -> str:
    """Differential flamegraph over the *after* profile, each frame colored by
    its change vs *before* (red = slower, green = faster). Needs folded stacks
    in both profiles (persisted by save_profile_json)."""
    if not after.folded:
        return ''

    class _N:
        __slots__ = ('c', 'kids')

        def __init__(self):
            self.c = 0
            self.kids: dict = {}

    def build(folded):
        root: dict = {}
        tot = 0
        for path, count in folded:
            tot += count
            node = root
            for name in path.split(';'):
                e = node.get(name)
                if e is None:
                    e = _N()
                    node[name] = e
                e.c += count
                node = e.kids
        return root, tot

    aroot, atot = build(after.folded)
    broot, _bt = build(before.folded) if before.folded else ({}, 0)
    if atot == 0:
        return ''
    pad = 2
    scale = (width - 2 * pad) / atot
    rects: list[str] = []
    maxd = [0]

    def emit(akids: dict, bkids: dict, offset: int, depth: int) -> None:
        maxd[0] = max(maxd[0], depth)
        x0 = offset
        for name, ch in sorted(akids.items(), key=lambda kv: (-kv[1].c, kv[0])):
            a = ch.c
            bnode = bkids.get(name) if bkids else None
            b = bnode.c if bnode else 0
            x = pad + x0 * scale
            w = a * scale
            y = depth * row_h
            d = a - b
            r = d / max(a, b, 1)
            if r > 0.05:
                col = f'hsl(0,70%,{max(45, 72 - int(r * 25))}%)'
            elif r < -0.05:
                col = f'hsl(140,55%,{min(75, 60 - int(r * 25))}%)'
            else:
                col = '#cbd5e1'
            label = name.split('\t')[0]
            title = html.escape(f'{label} — before {b}, after {a} ({d:+d} samples)')
            text = ''
            if w > 28:
                shown = label if len(label) * 6.5 < w else label[: max(1, int(w / 6.5))] + '…'
                text = (
                    f'<text x="{x + 3:.1f}" y="{y + row_h - 5}" font-size="11" '
                    f'fill="#1a1a1a" pointer-events="none">{html.escape(shown)}</text>'
                )
            rects.append(
                f'<g data-f0="{x0 / atot:.6f}" data-f1="{(x0 + a) / atot:.6f}">'
                f'<title>{title}</title>'
                f'<rect x="{x:.1f}" y="{y}" width="{max(w - 1, 0.5):.1f}" height="{row_h - 1}" '
                f'rx="1.5" fill="{col}" stroke="#fff" stroke-width="0.5"/>{text}</g>'
            )
            emit(ch.kids, bnode.kids if bnode else {}, x0, depth + 1)
            x0 += a

    emit(aroot, broot, 0, 0)
    height = (maxd[0] + 1) * row_h + pad
    svg = (
        f'<svg viewBox="0 0 {width} {height}" class="chart flame" role="img" '
        f'aria-label="Differential flamegraph" preserveAspectRatio="xMidYMin meet">{"".join(rects)}</svg>'
    )
    return (
        '<h2>Differential flamegraph <span class="muted">'
        '(after profile; red = slower, green = faster vs before)</span></h2>'
        + svg
        + _zoom_script('flame')
    )


def diff_profiles(before: ProfileResult, after: ProfileResult) -> list[FunctionDelta]:
    """Compare per-function self time between two profiles.

    Returns deltas sorted by largest absolute change first — the functions that
    moved most between (e.g.) before and after an optimization. Positive
    ``delta_ms`` is a regression (slower), negative is an improvement.
    """
    by_key: dict[tuple[str, str], list[float]] = {}
    for f in before.functions:
        by_key.setdefault((f.name, f.file), [0.0, 0.0])[0] = f.self_ms
    for f in after.functions:
        by_key.setdefault((f.name, f.file), [0.0, 0.0])[1] = f.self_ms

    deltas = [
        FunctionDelta(
            name=name,
            file=file,
            before_ms=b,
            after_ms=a,
            delta_ms=a - b,
        )
        for (name, file), (b, a) in by_key.items()
    ]
    deltas.sort(key=lambda d: abs(d.delta_ms), reverse=True)
    return deltas


def render_diff_html(
    before: ProfileResult,
    after: ProfileResult,
    title: str = 'rabbitinspect perf diff',
    limit: int = 100,
) -> str:
    """Render a self-contained before/after comparison report."""
    deltas = diff_profiles(before, after)[:limit]
    rows = []
    for d in deltas:
        if d.delta_ms > 0.05:
            cls, sign = 'reg', '+'
        elif d.delta_ms < -0.05:
            cls, sign = 'imp', ''
        else:
            cls, sign = 'flat', ''
        rows.append(
            '<tr>'
            f'<td class="name">{html.escape(d.name)}</td>'
            f'<td class="file">{html.escape(d.file)}</td>'
            f'<td class="num">{d.before_ms:.1f}</td>'
            f'<td class="num">{d.after_ms:.1f}</td>'
            f'<td class="num {cls}">{sign}{d.delta_ms:.1f}</td>'
            f'<td class="num {cls}">{sign}{d.delta_pct:.0f}%</td>'
            '</tr>'
        )
    dur_delta = after.duration_ms - before.duration_ms
    dur_cls = 'reg' if dur_delta > 0 else 'imp'
    flame = _diff_flamegraph_svg(before, after)
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
  .cards {{ display: flex; gap: 16px; margin-bottom: 24px; }}
  .card {{ background: #fff; border: 1px solid #e5e7eb; border-radius: 8px; padding: 12px 16px; min-width: 140px; }}
  .card .v {{ font-size: 22px; font-weight: 600; }}
  .card .k {{ color: #6b7280; font-size: 12px; text-transform: uppercase; }}
  table {{ width: 100%; border-collapse: collapse; background: #fff; }}
  th, td {{ text-align: left; padding: 6px 8px; border-bottom: 1px solid #f0f0f0; }}
  th {{ font-size: 12px; color: #6b7280; text-transform: uppercase; }}
  td.num {{ text-align: right; font-variant-numeric: tabular-nums; }}
  th.num {{ text-align: right; }}
  td.file {{ color: #6b7280; font-size: 12px; }}
  h2 {{ font-size: 15px; border-bottom: 2px solid #e5e7eb; padding-bottom: 6px; }}
  .chart {{ width: 100%; max-width: 1100px; background: #fff; border: 1px solid #e5e7eb; border-radius: 8px; }}
  .flame g:hover rect {{ stroke: #1f2937; stroke-width: 1; }}
  .muted {{ color: #9ca3af; }}
  .reg {{ color: #e03131; font-weight: 600; }}
  .imp {{ color: #2b8a3e; font-weight: 600; }}
  .flat {{ color: #9ca3af; }}
</style>
</head>
<body>
<header><h1>{html.escape(title)}</h1></header>
<main>
  <div class="cards">
    <div class="card"><div class="v">{before.duration_ms:.0f} ms</div><div class="k">Before</div></div>
    <div class="card"><div class="v">{after.duration_ms:.0f} ms</div><div class="k">After</div></div>
    <div class="card"><div class="v {dur_cls}">{dur_delta:+.0f} ms</div><div class="k">Total Δ</div></div>
  </div>
  {flame}
  <h2>Per-function change</h2>
  <table>
    <thead><tr><th>Function</th><th>File</th><th class="num">Before ms</th><th class="num">After ms</th><th class="num">Δ ms</th><th class="num">Δ %</th></tr></thead>
    <tbody>
    {chr(10).join(rows)}
    </tbody>
  </table>
</main>
</body>
</html>
"""


def save_profile_json(result: ProfileResult, path: str) -> None:
    """Persist the parts of a profile needed to diff it later (incl. folded
    stacks, so a differential flamegraph can be drawn)."""
    data = {
        'duration_ms': result.duration_ms,
        'sample_count': result.sample_count,
        'on_cpu_ms': result.on_cpu_ms,
        'off_cpu_ms': result.off_cpu_ms,
        'functions': [vars(f) for f in result.functions],
        'folded': result.folded,
    }
    with open(path, 'w', encoding='utf-8') as f:
        json.dump(data, f)


def load_profile_json(path: str) -> ProfileResult:
    """Load a profile previously written by :func:`save_profile_json`."""
    import dataclasses

    with open(path, encoding='utf-8') as f:
        data = json.load(f)
    if not isinstance(data, dict):
        raise ValueError(f'{path}: not a profile JSON object')
    # Tolerate version skew: keep only fields this FunctionStat knows about, so a
    # profile written by a newer/older rabbitinspect still loads.
    known = {fld.name for fld in dataclasses.fields(FunctionStat)}
    functions = [
        FunctionStat(**{k: v for k, v in d.items() if k in known})
        for d in data.get('functions', [])
    ]
    folded = [(s, c) for s, c in data.get('folded', [])]
    return ProfileResult(
        duration_ms=float(data.get('duration_ms', 0.0)),
        sample_count=int(data.get('sample_count', 0)),
        truncated=False,
        functions=functions,
        folded=folded,
        rss=[],
        on_cpu_ms=float(data.get('on_cpu_ms', 0.0)),
        off_cpu_ms=float(data.get('off_cpu_ms', 0.0)),
    )


def export_functions_csv(result: ProfileResult, path: str) -> None:
    """Write the per-function table as CSV (function,file,line,self_ms,total_ms,
    off_cpu_ms,self_pct)."""
    import csv

    with open(path, 'w', encoding='utf-8', newline='') as f:
        w = csv.writer(f)
        w.writerow(['function', 'file', 'line', 'self_ms', 'total_ms', 'off_cpu_ms', 'self_pct'])
        for fn in result.functions:
            w.writerow([fn.name, fn.file, fn.line, f'{fn.self_ms:.3f}', f'{fn.total_ms:.3f}',
                        f'{fn.off_cpu_ms:.3f}', f'{fn.self_pct:.3f}'])


# ── Launcher ────────────────────────────────────────────────────────────────


def profile_script(
    path: str,
    argv: list[str] | None = None,
    interval_ms: float = 5.0,
    max_depth: int = 256,
    trace_memory: bool = False,
) -> ProfileResult:
    """Run ``path`` as ``__main__`` under the profiler and return the result."""
    saved_argv = sys.argv
    sys.argv = [path, *(argv or [])]
    prof = Profiler(interval_ms=interval_ms, max_depth=max_depth, trace_memory=trace_memory)
    try:
        with prof:
            try:
                runpy.run_path(path, run_name='__main__')
            except SystemExit:
                pass
    finally:
        sys.argv = saved_argv
    assert prof.result is not None
    analyze_hotspots(prof.result)
    return prof.result


# ── exact per-line profiling (tracing) ───────────────────────────────────
#
# The sampler can only attribute time to lines it happens to catch; a line that
# runs in microseconds is essentially invisible. For a function you can edit,
# `@line_profile` instead uses sys.settrace to time *every* line execution
# exactly (high overhead — opt-in, for one function at a time).


def line_profile(fn):
    """Decorator: time every line of ``fn`` exactly via tracing (accumulates
    across calls). Read ``fn.line_stats`` or render with :func:`line_profile_html`.
    Heavy — instrument one function, not a whole app."""
    import functools

    code = fn.__code__
    stats: dict[int, list[float]] = {}  # lineno -> [total_seconds, hits]

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        import time

        last: list = [None, 0.0]  # [lineno, entered_at]

        def tracer(frame, event, arg):
            if frame.f_code is not code:
                return None  # don't trace callees — their time rolls up to the call site
            now = time.perf_counter()
            if event == 'line':
                if last[0] is not None:
                    stats.setdefault(last[0], [0.0, 0])[0] += now - last[1]
                stats.setdefault(frame.f_lineno, [0.0, 0])[1] += 1
                last[0] = frame.f_lineno
                last[1] = now
            elif event == 'return':
                if last[0] is not None:
                    stats.setdefault(last[0], [0.0, 0])[0] += now - last[1]
                    last[0] = None
            return tracer

        prev = sys.gettrace()
        sys.settrace(tracer)
        try:
            return fn(*args, **kwargs)
        finally:
            sys.settrace(prev)

    wrapper.line_stats = stats  # ty: ignore[unresolved-attribute]
    wrapper.profiled_function = fn  # ty: ignore[unresolved-attribute]
    return wrapper


def line_profile_result(wrapper) -> 'FunctionStat':
    """Turn a :func:`line_profile`-decorated wrapper's data into a FunctionStat
    (with ``line_times`` in ms, hottest first) so it renders like any function."""
    fn = wrapper.profiled_function
    stats: dict[int, list[float]] = wrapper.line_stats
    file = fn.__code__.co_filename
    line_times = sorted(
        ((ln, sec * 1000.0, hits) for ln, (sec, hits) in stats.items()),
        key=lambda t: -t[1],
    )
    total_ms = sum(t[1] for t in line_times)
    return FunctionStat(
        name=fn.__name__,
        file=file,
        self_ms=total_ms,
        total_ms=total_ms,
        self_pct=100.0,
        total_pct=100.0,
        line=fn.__code__.co_firstlineno,
        line_times=line_times,
    )


def line_profile_html(wrapper, title: str = 'rabbitinspect line profile') -> str:
    """Standalone HTML for one :func:`line_profile`-decorated function."""
    stat = line_profile_result(wrapper)
    result = ProfileResult(
        duration_ms=stat.self_ms,
        sample_count=0,
        truncated=False,
        functions=[stat],
        folded=[],
        rss=[],
    )
    return render_html(result, title=title)


# ── on-demand dump (signal handler) ───────────────────────────────────────


def install_dump_handler(out: str = 'rabbitinspect-dump.html', signum=None, app_root: str | None = None):
    """Dump an HTML report whenever ``signum`` (default SIGUSR1) is received,
    WITHOUT stopping the sampler. Lets you profile a long-running server and grab
    a report on demand: ``kill -USR1 <pid>``. Returns the previous handler."""
    import signal

    if signum is None:
        signum = getattr(signal, 'SIGUSR1', signal.SIGTERM)

    def _handler(_sig, _frame):
        if not _core.perf_running():
            return
        try:
            result = aggregate(_core.perf_snapshot())
            analyze_hotspots(result)
            with open(out, 'w', encoding='utf-8') as f:
                f.write(result.to_html(app_root=app_root))
        except Exception:
            pass

    return signal.signal(signum, _handler)


# ── asyncio task awareness ────────────────────────────────────────────────
#
# The Rust sampler reads `sys._current_frames()`, which only sees frames of
# OS threads that are *running* Python. A coroutine that is `await`-ing (the
# common case for an async server under load) has its frames detached from any
# thread, so it is invisible to that sampler. Here we sample the event loop's
# tasks directly via `Task.get_stack()`, capturing where suspended coroutines
# are parked. The captured stacks reuse the normal folded-stack format, so they
# flow through `aggregate` / the flamegraph / the HTML report unchanged.


def async_task_snapshot(loop=None) -> list[AsyncTaskInfo]:
    """Snapshot every asyncio task on ``loop`` (or the running loop)."""
    import asyncio

    try:
        tasks = asyncio.all_tasks(loop) if loop is not None else asyncio.all_tasks()
    except RuntimeError:
        return []  # no running loop

    infos: list[AsyncTaskInfo] = []
    for task in tasks:
        try:
            frames = task.get_stack()
        except Exception:
            continue
        # get_stack() is oldest-frame-first; we want leaf (await point) first.
        stack = [
            f'{fr.f_code.co_name}\t{fr.f_code.co_filename}\t{fr.f_lineno}'
            for fr in reversed(frames)
        ]
        name = task.get_name() if hasattr(task, 'get_name') else repr(task)
        infos.append(AsyncTaskInfo(name=name, state='done' if task.done() else 'pending', stack=stack))
    return infos


class AsyncSampler:
    """Background sampler for suspended coroutines on an asyncio loop.

    Complements :class:`Profiler`: where the CPU sampler catches the coroutine
    currently running on the loop thread, this catches all the *awaiting* ones.
    Call :meth:`stop` to get a :class:`ProfileResult` whose flamegraph shows the
    await hotspots.
    """

    def __init__(self, loop, interval_ms: float = 10.0):
        import threading

        self.loop = loop
        self.interval = interval_ms / 1000.0
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._start = 0.0
        self._frames: list[str] = []
        self._index: dict[str, int] = {}
        self._stacks: list[list[int]] = []
        self._ts: list[float] = []

    def _intern(self, entry: str) -> int:
        idx = self._index.get(entry)
        if idx is None:
            idx = len(self._frames)
            self._frames.append(entry)
            self._index[entry] = idx
        return idx

    def _run(self) -> None:
        import time

        while not self._stop.is_set():
            now = (time.perf_counter() - self._start) * 1000.0
            for info in async_task_snapshot(self.loop):
                # only count parked coroutines — running ones are the CPU sampler's job
                if info.state == 'pending' and info.stack:
                    self._stacks.append([self._intern(e) for e in info.stack])
                    self._ts.append(now)
            self._stop.wait(self.interval)

    def start(self) -> 'AsyncSampler':
        import threading
        import time

        self._start = time.perf_counter()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return self

    def stop(self) -> ProfileResult:
        import time

        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=1.0)
        raw = {
            'frames': self._frames,
            'stacks': self._stacks,
            'ts': self._ts,
            'tids': [],
            'rss': [],
            'spans': [],
            'queries': [],
            'duration_ms': (time.perf_counter() - self._start) * 1000.0,
            'sample_count': len(self._stacks),
            'truncated': False,
        }
        return aggregate(raw)


def profile_asyncio(main, interval_ms: float = 10.0) -> ProfileResult:
    """Run an async entrypoint and profile its awaiting coroutines.

    ``main`` is a zero-arg coroutine function (e.g. ``async def main(): ...``).
    Returns a :class:`ProfileResult` over the suspended-coroutine stacks — its
    flamegraph shows where the program spends time parked on ``await``.
    """
    import asyncio

    async def _driver() -> ProfileResult:
        sampler = AsyncSampler(asyncio.get_running_loop(), interval_ms=interval_ms).start()
        try:
            await main()
        finally:
            result = sampler.stop()
        return result

    return asyncio.run(_driver())


def _read_remote_rss(pid: int) -> float | None:
    """Resident set size (bytes) of a remote pid from ``/proc/<pid>/status``."""
    try:
        with open(f'/proc/{pid}/status') as f:
            for line in f:
                if line.startswith('VmRSS:'):
                    return float(line.split()[1]) * 1024.0  # kB → bytes
    except OSError:
        return None
    return None


def sample_remote(pid: int, duration_s: float = 3.0, interval_ms: float = 10.0) -> ProfileResult:
    """Sample one already-running process for ``duration_s`` and aggregate it.

    Reads the target's stacks out-of-process via :func:`_core.attach_sample`
    (no code changes in the target) and reuses the normal aggregation/report.
    Also samples the target's resident memory so the report's memory-over-time
    chart works for attached processes too.
    """
    return sample_remote_multi([pid], duration_s=duration_s, interval_ms=interval_ms)


def sample_remote_multi(
    pids: list[int], duration_s: float = 3.0, interval_ms: float = 10.0
) -> ProfileResult:
    """Sample several already-running processes at once and merge them.

    Useful for a forked server (gunicorn/uvicorn) where the work is spread over
    many worker pids: every tick samples each live worker. Threads are kept
    distinct across processes (the flame chart shows one band per worker thread),
    and RSS is summed across the workers.
    """
    import os
    import time

    frames: list[str] = []
    frame_index: dict[str, int] = {}
    stacks: list[list[int]] = []
    sample_ts: list[float] = []
    states: list[str] = []
    tids: list[int] = []
    rss: list[tuple[float, float]] = []

    def intern(entry: str) -> int:
        idx = frame_index.get(entry)
        if idx is None:
            idx = len(frames)
            frames.append(entry)
            frame_index[entry] = idx
        return idx

    alive = {pid: True for pid in pids}
    fails = {pid: 0 for pid in pids}
    start = time.perf_counter()
    while time.perf_counter() - start < duration_s:
        now = (time.perf_counter() - start) * 1000.0
        rss_total = 0.0
        got_rss = False
        for pid in pids:
            if not alive.get(pid):
                continue
            try:
                snapshot = _core.attach_sample(pid)
                fails[pid] = 0
            except OSError:
                # Transient read (a frame vanished mid-walk) — skip this pid this
                # tick. Drop the pid only if it actually exited or keeps failing.
                if not os.path.exists(f'/proc/{pid}'):
                    alive[pid] = False
                else:
                    fails[pid] += 1
                    if fails[pid] >= 100:
                        alive[pid] = False
                continue
            for thread in snapshot:
                stacks.append([intern(e) for e in thread['frames']])
                sample_ts.append(now)
                states.append(thread.get('state', 'R'))
                # keep threads distinct across processes for the per-thread view
                tid = int(thread.get('tid', 0))
                tids.append(pid * 1_000_000 + (tid % 1_000_000) if len(pids) > 1 else tid)
            r = _read_remote_rss(pid)
            if r is not None:
                rss_total += r
                got_rss = True
        if got_rss:
            rss.append((now, rss_total))
        if not any(alive.values()):
            break
        time.sleep(interval_ms / 1000.0)

    for pid in pids:
        _core.attach_forget(pid)  # drop cached interpreter resolution

    raw = {
        'frames': frames,
        'stacks': stacks,
        'ts': sample_ts,
        'tids': tids,
        'states': states,
        'rss': rss,
        'spans': [],
        'queries': [],
        'duration_ms': (time.perf_counter() - start) * 1000.0,
        'sample_count': len(stacks),
        'truncated': False,
    }
    result = aggregate(raw)
    analyze_hotspots(result)
    return result


def _attach_cli(
    pid: int,
    duration: float | None,
    out: str,
    interval_ms: float,
    app_root: str | None = None,
    also_pids: list[int] | None = None,
) -> int:
    """Inspect / sample an already-running process (F4)."""
    try:
        info = _core.attach_python_info(pid)
    except OSError as e:
        print(f'Cannot inspect pid {pid}: {e}', file=sys.stderr)
        return 1
    if not info['is_python']:
        print(f'pid {pid} does not look like a CPython process (no python/libpython mapping).', file=sys.stderr)
        return 1
    print(f'Attached to pid {pid}', file=sys.stderr)
    print(f'  interpreter: {info["interpreter_path"]}', file=sys.stderr)
    print(f'  image base:  0x{info["base"]:x}', file=sys.stderr)
    print(f'  mappings:    {info["maps_count"]}', file=sys.stderr)
    try:
        details = _core.attach_interpreter_info(pid)
        print(f'  python:      {details["version"]} (0x{details["version_hex"]:08x})', file=sys.stderr)
        print(f'  _PyRuntime:  0x{details["py_runtime_addr"]:x}', file=sys.stderr)
    except OSError as e:
        print(f'  python:      version unresolved ({e})', file=sys.stderr)
        return 1

    if duration is None:
        print('  (pass --duration to sample and write an HTML report)', file=sys.stderr)
        return 0

    pids = [pid, *(also_pids or [])]
    print(f'Sampling {"pids " + ",".join(map(str, pids)) if len(pids) > 1 else f"pid {pid}"} '
          f'for {duration:.1f}s …', file=sys.stderr)
    result = sample_remote_multi(pids, duration_s=duration, interval_ms=interval_ms)
    with open(out, 'w', encoding='utf-8') as f:
        f.write(result.to_html(app_root=app_root))
    print(f'  {result.sample_count} samples; report written to {out}', file=sys.stderr)
    if result.functions:
        print(f'  hottest: {result.functions[0].name} ({result.functions[0].self_pct:.1f}% self)', file=sys.stderr)
    return 0


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
    runp.add_argument('--speedscope', metavar='PATH', help='Also write a speedscope JSON profile')
    runp.add_argument('--json', metavar='PATH', dest='json_out', help='Also write a profile JSON for `perf diff`')
    runp.add_argument('--csv', metavar='PATH', dest='csv_out', help='Also write the function table as CSV')
    runp.add_argument('--interval', type=float, default=5.0, help='Sampling interval in ms')
    runp.add_argument('--max-depth', type=int, default=256, help='Maximum stack depth to walk')
    runp.add_argument('--memory', action='store_true', help='Also record per-function allocations (tracemalloc)')
    runp.add_argument('--app-root', metavar='PATH', help='Add a section with only your code under PATH')
    runp.add_argument('--app-only', action='store_true', help='Shorthand for --app-root <current directory>')
    runp.add_argument('script', help='Python script to profile')
    runp.add_argument('script_args', nargs=argparse.REMAINDER, help='Arguments passed to the script')

    attachp = sub.add_parser('attach', help='Inspect / sample an already-running process (Linux)')
    attachp.add_argument('--pid', type=int, required=True, help='Target process id')
    attachp.add_argument('--also-pid', type=int, action='append', default=[], metavar='PID',
                         help='Additional worker pids to sample together (repeatable)')
    attachp.add_argument('--duration', type=float, default=None, help='Seconds to sample (omit for info only)')
    attachp.add_argument('--out', default='rabbitinspect-perf.html', help='HTML report output path')
    attachp.add_argument('--interval', type=float, default=10.0, help='Sampling interval in ms')
    attachp.add_argument('--app-root', metavar='PATH', help='Add a section with only your code under PATH')
    attachp.add_argument('--app-only', action='store_true', help='Shorthand for --app-root <current directory>')

    diffp = sub.add_parser('diff', help='Compare two profile JSONs and write an HTML diff')
    diffp.add_argument('before', help='Baseline profile JSON (from `perf run --json`)')
    diffp.add_argument('after', help='New profile JSON')
    diffp.add_argument('--out', default='rabbitinspect-perf-diff.html', help='HTML diff output path')

    args = parser.parse_args(argv)

    def _resolve_app_root() -> str | None:
        if getattr(args, 'app_root', None):
            return args.app_root
        if getattr(args, 'app_only', False):
            return os.getcwd()
        return None

    if args.cmd == 'attach':
        return _attach_cli(
            args.pid, args.duration, args.out, args.interval, _resolve_app_root(), args.also_pid
        )
    if args.cmd == 'diff':
        try:
            before = load_profile_json(args.before)
            after = load_profile_json(args.after)
        except FileNotFoundError as e:
            print(f'Error: profile JSON not found: {e.filename}', file=sys.stderr)
            return 1
        except (ValueError, TypeError) as e:  # JSONDecodeError, missing/extra fields
            print(f'Error: could not read profile JSON: {e}', file=sys.stderr)
            return 1
        with open(args.out, 'w', encoding='utf-8') as f:
            f.write(render_diff_html(before, after))
        deltas = diff_profiles(before, after)
        if deltas and deltas[0].delta_ms != 0.0:
            top = deltas[0]
            verb = 'slower' if top.delta_ms > 0 else 'faster'
            print(f'Biggest change: {top.name} {abs(top.delta_ms):.1f} ms {verb}', file=sys.stderr)
        print(f'Diff written to {args.out}', file=sys.stderr)
        return 0
    if args.cmd == 'run':
        result = profile_script(
            args.script,
            args.script_args,
            interval_ms=args.interval,
            max_depth=args.max_depth,
            trace_memory=args.memory,
        )
        with open(args.out, 'w', encoding='utf-8') as f:
            f.write(result.to_html(app_root=_resolve_app_root()))
        if args.speedscope:
            import json as _json

            with open(args.speedscope, 'w', encoding='utf-8') as f:
                _json.dump(to_speedscope(result), f)
            print(f'Speedscope profile written to {args.speedscope}', file=sys.stderr)
        if args.json_out:
            save_profile_json(result, args.json_out)
            print(f'Profile JSON written to {args.json_out}', file=sys.stderr)
        if args.csv_out:
            export_functions_csv(result, args.csv_out)
            print(f'Functions CSV written to {args.csv_out}', file=sys.stderr)
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
