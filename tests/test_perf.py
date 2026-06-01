"""Tests for the sampling runtime profiler (F1)."""

import sys

import pytest
from rabbitinspect import _core
from rabbitinspect.perf import (
    FunctionStat,
    Profiler,
    ProfileResult,
    aggregate,
    profile_script,
    render_html,
)


@pytest.fixture(autouse=True)
def _ensure_sampler_stopped():
    # The sampler is process-global; never let a failed test leak it.
    yield
    if _core.perf_running():
        _core.perf_stop()


def _burn(seconds: float = 0.4) -> int:
    """CPU-bound work that stays in one Python frame named ``_burn``."""
    import time

    end = time.perf_counter() + seconds
    x = 0
    i = 0
    while time.perf_counter() < end:
        for _ in range(10000):
            x += i * i
            i += 1
    return x


# ── sampler lifecycle ───────────────────────────────────────────────────────


def test_running_flag_toggles():
    assert _core.perf_running() is False
    _core.perf_start(5.0, 64)
    try:
        assert _core.perf_running() is True
    finally:
        _core.perf_stop()
    assert _core.perf_running() is False


def test_double_start_raises():
    _core.perf_start(5.0, 64)
    try:
        with pytest.raises(RuntimeError):
            _core.perf_start(5.0, 64)
    finally:
        _core.perf_stop()


def test_stop_without_start_raises():
    assert _core.perf_running() is False
    with pytest.raises(RuntimeError):
        _core.perf_stop()


# ── sampling correctness ─────────────────────────────────────────────────────


def test_profiler_detects_hot_function():
    with Profiler(interval_ms=1.0) as prof:
        _burn(0.4)
    result = prof.result
    assert result is not None
    assert result.sample_count > 0
    assert result.duration_ms > 0
    names = [f.name for f in result.functions]
    assert '_burn' in names
    # _burn should dominate self time by a wide margin.
    hottest = result.functions[0]
    assert hottest.name == '_burn'
    assert hottest.self_pct > 40.0


def test_raw_structure():
    _core.perf_start(2.0, 64)
    _burn(0.15)
    raw = _core.perf_stop()
    assert set(raw) >= {
        'frames',
        'stacks',
        'ts',
        'tids',
        'rss',
        'duration_ms',
        'sample_count',
        'truncated',
    }
    assert raw['sample_count'] == len(raw['stacks'])
    assert len(raw['ts']) == len(raw['stacks'])


@pytest.mark.skipif(not sys.platform.startswith('linux'), reason='RSS timeline is Linux-only for now')
def test_rss_timeline_collected_on_linux():
    with Profiler(interval_ms=2.0) as prof:
        _burn(0.2)
    result = prof.result
    assert len(result.rss) > 0
    assert result.peak_rss_bytes > 0


# ── aggregation ──────────────────────────────────────────────────────────────


def _empty_raw() -> dict:
    return {
        'frames': [],
        'stacks': [],
        'ts': [],
        'tids': [],
        'rss': [],
        'duration_ms': 0.0,
        'sample_count': 0,
        'truncated': False,
    }


def test_aggregate_empty():
    result = aggregate(_empty_raw())
    assert result.functions == []
    assert result.folded == []
    assert result.sample_count == 0
    assert result.peak_rss_bytes == 0.0


def test_aggregate_self_vs_total():
    # Two samples: stack a<-b (leaf b) and a<-b<-c (leaf c).
    # frame format is "func\tfile\tline".
    raw = {
        'frames': ['a\tx.py\t1', 'b\tx.py\t2', 'c\tx.py\t3'],
        'stacks': [[1, 0], [2, 1, 0]],  # leaf-first
        'ts': [1.0, 2.0],
        'tids': [1, 1],
        'rss': [[1.0, 1000.0], [2.0, 2000.0]],
        'duration_ms': 10.0,
        'sample_count': 2,
        'truncated': False,
    }
    result = aggregate(raw)
    by_name = {f.name: f for f in result.functions}
    # a appears in both stacks (total) but is never the leaf (self 0).
    assert by_name['a'].total_pct == pytest.approx(100.0)
    assert by_name['a'].self_pct == pytest.approx(0.0)
    # b is leaf once, present in both.
    assert by_name['b'].self_pct == pytest.approx(50.0)
    assert by_name['b'].total_pct == pytest.approx(100.0)
    # c is leaf once, present once.
    assert by_name['c'].self_pct == pytest.approx(50.0)
    assert by_name['c'].total_pct == pytest.approx(50.0)
    assert result.peak_rss_bytes == 2000.0


def test_aggregate_off_cpu_split():
    # 4 samples: leaf 'work' on-CPU twice, leaf 'wait' off-CPU (sleeping) twice.
    raw = {
        'frames': ['work\tx.py\t1', 'wait\tx.py\t2'],
        'stacks': [[0], [0], [1], [1]],
        'ts': [1.0, 2.0, 3.0, 4.0],
        'tids': [1, 1, 1, 1],
        'states': ['R', 'R', 'S', 'D'],
        'rss': [],
        'duration_ms': 100.0,
        'sample_count': 4,
        'truncated': False,
    }
    result = aggregate(raw)
    # half the wall time was spent waiting.
    assert result.off_cpu_ms == pytest.approx(50.0)
    assert result.on_cpu_ms == pytest.approx(50.0)
    by_name = {f.name: f for f in result.functions}
    # 'work' is pure on-CPU; 'wait' is fully off-CPU.
    assert by_name['work'].off_cpu_ms == pytest.approx(0.0)
    assert by_name['wait'].off_cpu_ms == pytest.approx(50.0)


def test_aggregate_no_states_means_on_cpu():
    # Without a 'states' array (in-process sampler) nothing is marked off-CPU.
    result = aggregate(_empty_raw())
    assert result.off_cpu_ms == 0.0
    assert result.on_cpu_ms == 0.0


# ── HTML report ──────────────────────────────────────────────────────────────


def _synthetic_result() -> ProfileResult:
    return ProfileResult(
        duration_ms=123.0,
        sample_count=42,
        truncated=False,
        functions=[FunctionStat('compute', 'app.py', 80.0, 100.0, 65.0, 81.0)],
        folded=[('main;compute', 30), ('main', 12)],
        rss=[(0.0, 10 * 1024 * 1024), (50.0, 20 * 1024 * 1024)],
    )


def test_render_html_sections():
    html = render_html(_synthetic_result())
    assert '<table' in html
    assert 'Top functions' in html
    assert 'Memory over time' in html
    assert 'Folded stacks' in html
    assert 'compute' in html
    assert 'rabbitinspect-perf-data' in html  # embedded JSON payload
    assert '20.0 MB peak' in html  # RSS chart label


def test_render_html_flamegraph():
    html = render_html(_synthetic_result())
    assert 'Flamegraph' in html
    assert 'class="chart flame"' in html
    # one rect per tree node: main, compute → at least 2 rects
    assert html.count('<rect') >= 2
    # frame label + sample count appear in a <title> tooltip
    assert 'samples' in html


def test_flamegraph_widths_proportional():
    from rabbitinspect.perf import _flamegraph_svg

    # main=100 samples total (60 self via 'main', 40 through 'main;compute')
    r = ProfileResult(
        duration_ms=100.0,
        sample_count=100,
        truncated=False,
        functions=[],
        folded=[('main', 60), ('main;compute', 40)],
        rss=[],
    )
    svg = _flamegraph_svg(r, width=1000)
    import re

    widths = {}
    for g in re.findall(r'<g [^>]*>.*?</g>', svg):
        name = re.search(r'<title>([^ ]+)', g)
        w = re.search(r'width="([\d.]+)"', g)
        if name and w:
            widths[name.group(1)] = float(w.group(1))
    # main spans full width; compute is 40% of it.
    assert widths['main'] > 980
    assert 380 < widths['compute'] < 420


def test_flamegraph_interactive_zoom_data():
    from rabbitinspect.perf import _flamegraph_svg

    r = ProfileResult(
        100.0, 100, False, [],
        [('main', 60), ('main;compute', 40)], [],
    )
    svg = _flamegraph_svg(r, width=1000)
    # each frame carries normalized [f0,f1] fractions for the zoom script
    assert 'data-f0=' in svg and 'data-f1=' in svg
    # the zoom script + click handlers are embedded (self-contained, no deps)
    assert '<script>' in svg
    assert 'addEventListener("click"' in svg
    # full-width root frame spans the whole [0,1] range
    assert 'data-f0="0.000000" data-f1="1.000000"' in svg
    # regression: the script must live in HTML context (after </svg>), NOT inside
    # the SVG — document.currentScript is null for SVG <script> elements.
    assert 'currentScript' not in svg
    assert svg.index('</svg>') < svg.index('<script>')
    assert 'querySelectorAll("svg.flame")' in svg


def test_flamegraph_empty():
    from rabbitinspect.perf import _flamegraph_svg

    r = ProfileResult(100.0, 0, False, [], [], [])
    assert 'No stacks' in _flamegraph_svg(r)


def test_flamegraph_escapes_names():
    from rabbitinspect.perf import _flamegraph_svg

    r = ProfileResult(10.0, 1, False, [], [('<script>x', 1)], [])
    svg = _flamegraph_svg(r)
    assert '<script>x' not in svg
    assert '&lt;script&gt;x' in svg


def test_diff_profiles_orders_by_change():
    from rabbitinspect.perf import diff_profiles

    before = ProfileResult(
        100.0, 10, False,
        [FunctionStat('a', 'x.py', 100.0, 100.0, 0, 0), FunctionStat('b', 'x.py', 50.0, 50.0, 0, 0)],
        [], [],
    )
    after = ProfileResult(
        70.0, 10, False,
        [FunctionStat('a', 'x.py', 40.0, 40.0, 0, 0),
         FunctionStat('b', 'x.py', 50.0, 50.0, 0, 0),
         FunctionStat('c', 'x.py', 30.0, 30.0, 0, 0)],
        [], [],
    )
    deltas = diff_profiles(before, after)
    by = {d.name: d for d in deltas}
    # 'a' improved most (-60), then 'c' is new (+30), 'b' unchanged (0).
    assert [d.name for d in deltas] == ['a', 'c', 'b']
    assert by['a'].delta_ms == pytest.approx(-60.0)
    assert by['a'].delta_pct == pytest.approx(-60.0)
    assert by['c'].before_ms == 0.0
    assert by['c'].delta_ms == pytest.approx(30.0)
    assert by['c'].delta_pct == pytest.approx(100.0)
    assert by['b'].delta_ms == pytest.approx(0.0)


def test_render_diff_html():
    from rabbitinspect.perf import render_diff_html

    before = ProfileResult(100.0, 10, False, [FunctionStat('slow', 'x.py', 100.0, 100.0, 0, 0)], [], [])
    after = ProfileResult(40.0, 10, False, [FunctionStat('slow', 'x.py', 40.0, 40.0, 0, 0)], [], [])
    html_out = render_diff_html(before, after)
    assert 'Before' in html_out and 'After' in html_out
    assert 'slow' in html_out
    assert 'imp' in html_out  # improvement class for the -60 ms change
    assert '-60.0' in html_out
    assert '-60 ms' in html_out  # total duration delta card


def test_render_diff_html_escapes():
    from rabbitinspect.perf import render_diff_html

    r = ProfileResult(1.0, 1, False, [FunctionStat('<script>', 'a.py', 1.0, 1.0, 0, 0)], [], [])
    out = render_diff_html(r, r)
    assert '<script>' not in out
    assert '&lt;script&gt;' in out


def test_render_html_truncated_warning():
    r = _synthetic_result()
    r.truncated = True
    assert 'truncated' in render_html(r).lower()


def test_html_escapes_function_names():
    r = _synthetic_result()
    r.functions = [FunctionStat('<script>evil', 'a.py', 1.0, 1.0, 1.0, 1.0)]
    html = render_html(r)
    assert '<script>evil' not in html
    assert '&lt;script&gt;evil' in html


# ── launcher ─────────────────────────────────────────────────────────────────


def test_profile_script(tmp_path):
    script = tmp_path / 'work.py'
    script.write_text(
        'import time\n'
        'def busy():\n'
        '    end = time.perf_counter() + 0.3\n'
        '    x = 0\n'
        '    while time.perf_counter() < end:\n'
        '        x += 1\n'
        '    return x\n'
        'busy()\n'
    )
    result = profile_script(str(script), interval_ms=1.0)
    assert result.sample_count > 0
    assert any(f.name == 'busy' for f in result.functions)
    assert not _core.perf_running()
