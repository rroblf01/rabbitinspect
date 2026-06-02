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


def test_aggregate_time_not_diluted_by_threads():
    # 4 ticks, 2 threads each tick (8 stacks). Same timestamp per tick.
    # Thread A always runs 'work'; thread B always runs 'idle'.
    raw = {
        'frames': ['work\tx.py\t1', 'idle\ty.py\t1'],
        'stacks': [[0], [1], [0], [1], [0], [1], [0], [1]],
        'ts': [0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0],  # shared per tick
        'tids': [1, 2, 1, 2, 1, 2, 1, 2],
        'rss': [],
        'duration_ms': 100.0,
        'sample_count': 8,
        'truncated': False,
    }
    result = aggregate(raw)
    by = {f.name: f for f in result.functions}
    # 'work' ran the entire window on its thread → full duration, NOT 100/2.
    # (the bug normalized by len(stacks)=8, halving it to 50 ms.)
    assert by['work'].self_ms == pytest.approx(100.0)
    assert by['work'].self_pct == pytest.approx(100.0)
    assert by['idle'].self_ms == pytest.approx(100.0)


def test_aggregate_reports_hot_line():
    # 'get' sampled mostly at line 10, once at line 8 → representative line = 10.
    raw = {
        'frames': ['get\turls.py\t10', 'get\turls.py\t8'],
        'stacks': [[0], [0], [0], [1]],
        'ts': [0.0, 1.0, 2.0, 3.0],
        'tids': [1, 1, 1, 1],
        'rss': [],
        'duration_ms': 40.0,
        'sample_count': 4,
        'truncated': False,
    }
    result = aggregate(raw)
    get = next(f for f in result.functions if f.name == 'get')
    assert get.line == 10
    # the line shows up in the rendered file cell as file:line
    assert 'urls.py:10' in render_html(result)


def test_aggregate_line_times_breakdown():
    # 'get' sampled at line 10 (×3) and line 8 (×1) as the leaf.
    raw = {
        'frames': ['get\turls.py\t10', 'get\turls.py\t8'],
        'stacks': [[0], [0], [0], [1]],
        'ts': [0.0, 1.0, 2.0, 3.0],
        'tids': [1, 1, 1, 1],
        'rss': [],
        'duration_ms': 40.0,
        'sample_count': 4,
        'truncated': False,
    }
    result = aggregate(raw)
    get = next(f for f in result.functions if f.name == 'get')
    # per-line self time, hottest first; weight = 40ms / 4 ticks = 10ms per sample
    assert get.line_times[0] == (10, pytest.approx(30.0), 3)
    assert get.line_times[1] == (8, pytest.approx(10.0), 1)

    html = result.to_html()
    # the row is expandable and the per-line detail table is embedded
    assert 'frow fn"' in html
    assert 'fndetail' in html
    assert 'table class="lines"' in html or 'class="lines"' in html
    # the toggle script is wired
    assert 'tr.fn' in html


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
    assert 'functions by self time' in html
    assert 'Memory over time' in html
    assert 'Folded stacks' in html
    assert 'compute' in html
    assert 'rabbitinspect-perf-data' in html  # embedded JSON payload
    assert '20.0 MB peak' in html  # RSS chart label


def _two_call_raw():
    # main runs the whole time; get is called twice (10..30 and 40..50).
    return {
        'frames': ['main\ta.py\t1', 'get\ta.py\t5'],
        'stacks': [[0], [1, 0], [1, 0], [0], [1, 0], [0]],  # leaf-first
        'ts': [0.0, 10.0, 20.0, 30.0, 40.0, 50.0],
        'tids': [1, 1, 1, 1, 1, 1],
        'rss': [],
        'duration_ms': 60.0,
        'sample_count': 6,
        'truncated': False,
    }


def test_build_segments_counts_each_call():
    from rabbitinspect.perf import _build_segments

    segs = _build_segments(_two_call_raw())
    gets = [s for s in segs if s.func == 'get']
    # 'get' was called twice → two separate segments (time order, not aggregate)
    assert len(gets) == 2
    assert sorted(round(s.duration_ms) for s in gets) == [10, 20]
    main = [s for s in segs if s.func == 'main'][0]
    assert main.duration_ms >= 50  # open the whole run


def test_call_timings_section_counts_and_durations():
    from rabbitinspect.perf import _build_segments, _call_timings_section

    html = _call_timings_section(_build_segments(_two_call_raw()))
    assert 'Call timings' in html
    # get: 2 calls, total 30, avg 15, max 20
    row = html.split('get')[1].split('</tr>')[0]
    assert '>2<' in row  # call count
    assert '20.0' in row  # max ms


def test_flamechart_in_report():
    result = aggregate(_two_call_raw())
    html = result.to_html()
    assert 'Flame chart' in html
    assert 'class="chart flamechart"' in html
    assert 'thread 1' in html
    assert 'data-f0' in html
    # hover tooltip carries the per-call duration
    assert 'ms' in html
    # its own zoom handler is wired
    assert 'querySelectorAll("svg.flamechart")' in html


def test_flamechart_absent_without_timestamps():
    from rabbitinspect.perf import _build_segments

    raw = _two_call_raw()
    raw['ts'] = []  # no timestamps → no time-order view
    assert _build_segments(raw) == []
    result = aggregate(raw)
    assert 'Flame chart' not in result.to_html()


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


def test_app_functions_filter():
    from rabbitinspect.perf import _is_app_frame, render_html

    root = '/home/me/proj'
    # app code under root, not in a venv
    assert _is_app_frame('/home/me/proj/app/views.py', root)
    # dependency inside the project's venv → not app code
    assert not _is_app_frame('/home/me/proj/.venv/lib/python3.14/site-packages/django/x.py', root)
    # stdlib outside root → not app code
    assert not _is_app_frame('/usr/lib/python3.14/socket.py', root)
    # synthetic frames → not app code
    assert not _is_app_frame('<frozen importlib._bootstrap>', root)

    result = ProfileResult(
        100.0, 10, False,
        [
            FunctionStat('readinto', '/usr/lib/python3.14/socket.py', 60.0, 60.0, 60.0, 60.0),
            FunctionStat('get', '/home/me/proj/app/views.py', 5.0, 5.0, 0, 5.0),
        ],
        [], [],
    )
    # without app_root: only the full table, no app section
    assert 'Top application functions' not in render_html(result)
    # with app_root: the app section appears and lists only the user's function
    html = render_html(result, app_root='/home/me/proj')
    assert 'Top application functions' in html
    app_part = html.split('Top application functions')[1].split('All functions by self time')[0]
    assert 'get' in app_part
    assert 'readinto' not in app_part  # stdlib excluded from the app section


def test_hotspots_filtered_by_app_root():
    from rabbitinspect.perf import HotspotLint, render_html

    result = ProfileResult(
        100.0, 10, False, [], [], [],
        hotspot_lints=[
            HotspotLint('myview', '/home/me/proj/app/views.py', 50.0,
                        [{'code': 'RAB001', 'line': 3, 'message': 'x'}]),
            HotspotLint('select', '/usr/lib/python3.14/selectors.py', 40.0,
                        [{'code': 'RAB002', 'line': 9, 'message': 'y'}]),
            HotspotLint('inner', '/home/me/proj/.venv/lib/python3.14/site-packages/django/x.py', 30.0,
                        [{'code': 'RAB003', 'line': 1, 'message': 'z'}]),
        ],
    )
    # no app_root → all hotspots shown (current behavior)
    full = render_html(result)
    assert 'myview' in full and 'select' in full and 'inner' in full
    # app_root → only the user's own code, no stdlib, no .venv
    scoped = render_html(result, app_root='/home/me/proj')
    assert 'myview' in scoped
    assert 'selectors.py' not in scoped
    assert 'site-packages' not in scoped


def test_app_functions_section_empty():
    from rabbitinspect.perf import render_html

    result = ProfileResult(
        100.0, 10, False,
        [FunctionStat('readinto', '/usr/lib/python3.14/socket.py', 60.0, 60.0, 60.0, 60.0)],
        [], [],
    )
    html = render_html(result, app_root='/home/me/proj')
    assert 'No samples landed in your own code' in html


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


# ── 2.0 additions: line_profile / snapshot / dump / csv / diff flame / endpoints ──


def test_line_profile_exact_per_line():
    from rabbitinspect.perf import line_profile, line_profile_result

    @line_profile
    def work(n):
        total = 0
        for i in range(n):
            total += i * i
        return total

    work(50000)
    stat = line_profile_result(work)
    assert stat.line_times  # per-line data captured
    # the loop body line is hit n times
    max_hits = max(h for _ln, _ms, h in stat.line_times)
    assert max_hits >= 50000


def test_line_profile_html():
    from rabbitinspect.perf import line_profile, line_profile_html

    @line_profile
    def f():
        x = 0
        for _ in range(1000):
            x += 1
        return x

    f()
    html_out = line_profile_html(f)
    assert 'line profile' in html_out.lower()
    assert 'frow fn"' in html_out  # expandable per-line row


def test_perf_snapshot_while_running():
    _core.perf_start(2.0, 64)
    try:
        s = 0
        for i in range(300000):
            s += i
        raw = _core.perf_snapshot()
        assert _core.perf_running()  # snapshot does NOT stop
        assert 'stacks' in raw and 'frames' in raw
    finally:
        _core.perf_stop()


@pytest.mark.skipif(not sys.platform.startswith('linux'), reason='SIGUSR1 dump is POSIX')
def test_install_dump_handler(tmp_path):
    import os
    import signal
    import time

    from rabbitinspect.perf import install_dump_handler

    out = tmp_path / 'dump.html'
    prev = install_dump_handler(out=str(out))
    try:
        _core.perf_start(2.0, 64)
        s = 0
        for i in range(300000):
            s += i
        os.kill(os.getpid(), signal.SIGUSR1)
        time.sleep(0.1)
        assert _core.perf_running()  # still running after dump
        assert out.exists()
        assert 'functions by self time' in out.read_text(encoding='utf-8')
    finally:
        if _core.perf_running():
            _core.perf_stop()
        signal.signal(signal.SIGUSR1, prev)


def test_export_functions_csv(tmp_path):
    from rabbitinspect.perf import export_functions_csv

    r = ProfileResult(100.0, 10, False, [FunctionStat('f', 'a.py', 50.0, 60.0, 50.0, 60.0, line=3)], [], [])
    path = tmp_path / 'fns.csv'
    export_functions_csv(r, str(path))
    text = path.read_text()
    assert 'function,file,line,self_ms' in text
    assert 'f,a.py,3' in text


def test_diff_flamegraph_in_diff_html():
    from rabbitinspect.perf import render_diff_html

    before = ProfileResult(100.0, 10, False, [], [('main;slow', 80), ('main', 20)], [])
    after = ProfileResult(60.0, 10, False, [], [('main;slow', 30), ('main', 30)], [])
    html_out = render_diff_html(before, after)
    assert 'Differential flamegraph' in html_out
    assert 'class="chart flame"' in html_out
    assert 'querySelectorAll("svg.flame")' in html_out


def test_report_toolbar_search_dark_csv():
    r = aggregate({
        'frames': ['f\ta.py\t1'], 'stacks': [[0]], 'ts': [0.0], 'tids': [1],
        'rss': [], 'duration_ms': 10.0, 'sample_count': 1, 'truncated': False,
    })
    html_out = r.to_html()
    assert 'id="fsearch"' in html_out          # search box
    assert 'rabFilter' in html_out
    assert "classList.toggle('dark')" in html_out  # dark mode
    assert 'rabExportCsv' in html_out          # csv download


def test_endpoint_flamegraphs():
    # samples taken during a request window to /v, plus the span itself
    raw = {
        'frames': ['view\turls.py\t5', 'wsgi\tx.py\t1'],
        'stacks': [[0, 1], [0, 1], [0, 1]],
        'ts': [1.0, 2.0, 3.0],
        'tids': [1, 1, 1],
        'states': ['R', 'R', 'R'],
        'rss': [],
        'spans': [{'method': 'GET', 'route': '/v', 'status': 200, 'start_ms': 0.0, 'end_ms': 5.0}],
        'queries': [],
        'duration_ms': 5.0,
        'sample_count': 3,
        'truncated': False,
    }
    result = aggregate(raw)
    html_out = result.to_html()
    assert 'Per-endpoint flamegraphs' in html_out
    assert 'GET /v' in html_out
