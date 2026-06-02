"""End-to-end tests for the 2.0 runtime profiler and regression tests for the
release bug-review fixes.

Covers:
- WSGI middleware against a *lazy/streaming* body (span recorded at consume time,
  status captured from a deferred ``start_response``, underlying ``close()`` run).
- ``perf diff`` CLI against missing / malformed / version-skewed profile JSON.
- ``Profiler`` not leaking tracemalloc when the sampler refuses to start.
- Repeated start/stop cycles (the Arc-not-leak fix) staying functional.
- A full ``perf run`` → HTML report → speedscope → JSON → diff pipeline.
"""

import json
import sys

import pytest
from rabbitinspect import _core
from rabbitinspect.perf import (
    Profiler,
    load_profile_json,
    run_perf_cli,
    save_profile_json,
)
from rabbitinspect.perf_web import WSGIProfilerMiddleware


@pytest.fixture(autouse=True)
def _ensure_sampler_stopped():
    yield
    if _core.perf_running():
        _core.perf_stop()
    import tracemalloc

    if tracemalloc.is_tracing():
        tracemalloc.stop()


# ── WSGI streaming / lazy body (regression for the lazy-iterable timing bug) ──


def test_wsgi_span_recorded_at_body_consumption():
    """The span must be recorded only once the server consumes the body, not when
    the app callable returns its (lazy) iterable."""

    def streaming_app(environ, start_response):
        def body():
            # start_response is deferred until the body is first iterated — a
            # perfectly legal WSGI pattern. The old middleware recorded the span
            # before this ran, so it captured status 0 and ~0 ms.
            start_response('200 OK', [('Content-Type', 'text/plain')])
            yield b'chunk-1'
            yield b'chunk-2'

        return body()

    mw = WSGIProfilerMiddleware(streaming_app)
    _core.perf_start(5.0, 64)
    try:
        it = mw({'REQUEST_METHOD': 'GET', 'PATH_INFO': '/stream'}, lambda *a, **k: None)
        # Before consuming the body: no span yet.
        assert len(_core.perf_snapshot()['spans']) == 0
        body = b''.join(it)
        # After consuming: exactly one span, with the deferred status captured.
        spans = _core.perf_snapshot()['spans']
        assert body == b'chunk-1chunk-2'
        assert len(spans) == 1
        assert spans[0]['status'] == 200
        assert spans[0]['route'] == '/stream'
    finally:
        _core.perf_stop()


def test_wsgi_forwards_close():
    """The wrapped iterable must forward close() to the app's body so response
    cleanup hooks still fire."""

    class ClosableBody:
        def __init__(self):
            self.closed = False

        def __iter__(self):
            return iter([b'a', b'b'])

        def close(self):
            self.closed = True

    captured = {}

    def app(environ, start_response):
        start_response('200 OK', [])
        body = ClosableBody()
        captured['body'] = body
        return body

    mw = WSGIProfilerMiddleware(app)
    _core.perf_start(5.0, 64)
    try:
        list(mw({'REQUEST_METHOD': 'GET', 'PATH_INFO': '/c'}, lambda *a, **k: None))
    finally:
        _core.perf_stop()
    assert captured['body'].closed is True


def test_wsgi_records_span_on_early_close():
    """If the server closes the body early (e.g. client disconnect), the span is
    still recorded via the generator's finally."""

    def app(environ, start_response):
        start_response('200 OK', [])
        return iter([b'x', b'y', b'z'])

    mw = WSGIProfilerMiddleware(app)
    _core.perf_start(5.0, 64)
    try:
        it = mw({'REQUEST_METHOD': 'GET', 'PATH_INFO': '/early'}, lambda *a, **k: None)
        next(iter(it))  # consume one chunk
        it.close()  # server abandons the rest
        spans = _core.perf_snapshot()['spans']
        assert len(spans) == 1
        assert spans[0]['route'] == '/early'
    finally:
        _core.perf_stop()


# ── perf diff CLI hardening ───────────────────────────────────────────────────


def test_perf_diff_missing_file_exits_cleanly(tmp_path, capsys):
    rc = run_perf_cli(['diff', str(tmp_path / 'nope.json'), str(tmp_path / 'also-nope.json'),
                       '--out', str(tmp_path / 'd.html')])
    assert rc == 1
    err = capsys.readouterr().err
    assert 'not found' in err
    assert not (tmp_path / 'd.html').exists()


def test_perf_diff_malformed_json_exits_cleanly(tmp_path, capsys):
    bad = tmp_path / 'bad.json'
    bad.write_text('{not valid json')
    good = tmp_path / 'good.json'
    good.write_text('{"duration_ms": 1.0, "sample_count": 0, "functions": [], "folded": []}')
    rc = run_perf_cli(['diff', str(bad), str(good), '--out', str(tmp_path / 'd.html')])
    assert rc == 1
    assert 'could not read profile JSON' in capsys.readouterr().err


def test_perf_diff_not_an_object_exits_cleanly(tmp_path, capsys):
    arr = tmp_path / 'arr.json'
    arr.write_text('[1, 2, 3]')
    other = tmp_path / 'o.json'
    other.write_text('{"functions": []}')
    rc = run_perf_cli(['diff', str(arr), str(other), '--out', str(tmp_path / 'd.html')])
    assert rc == 1
    assert 'could not read profile JSON' in capsys.readouterr().err


# ── load_profile_json version-skew tolerance ──────────────────────────────────


def test_load_profile_json_ignores_unknown_fields(tmp_path):
    """A profile written by a future rabbitinspect (extra FunctionStat fields)
    must still load instead of raising TypeError."""
    path = tmp_path / 'future.json'
    path.write_text(json.dumps({
        'duration_ms': 10.0,
        'sample_count': 3,
        'functions': [{
            'name': 'f', 'file': 'a.py', 'self_ms': 1.0, 'total_ms': 2.0,
            'self_pct': 50.0, 'total_pct': 100.0,
            'a_field_from_the_future': 'whatever',  # unknown → must be dropped
        }],
        'folded': [],
    }))
    result = load_profile_json(str(path))
    assert result.sample_count == 3
    assert result.functions[0].name == 'f'


def test_load_profile_json_roundtrip_with_extra_skew(tmp_path):
    """Round-trip then inject a stray key; load must survive."""
    src = tmp_path / 'demo.py'
    src.write_text('x = sum(range(100000))\n')
    rc = run_perf_cli(['run', '--out', str(tmp_path / 'r.html'),
                       '--json', str(tmp_path / 'p.json'), '--interval', '1', str(src)])
    assert rc == 0
    data = json.loads((tmp_path / 'p.json').read_text())
    for fn in data['functions']:
        fn['unexpected'] = 1
    (tmp_path / 'p.json').write_text(json.dumps(data))
    loaded = load_profile_json(str(tmp_path / 'p.json'))
    assert loaded.duration_ms > 0


# ── Profiler / tracemalloc leak guard ─────────────────────────────────────────


def test_profiler_does_not_leak_tracemalloc_when_start_fails():
    """If perf_start raises (another sampler already active), a trace_memory
    Profiler must not leave tracemalloc running."""
    import tracemalloc

    assert not tracemalloc.is_tracing()
    _core.perf_start(5.0, 64)  # occupy the single global sampler slot
    try:
        with pytest.raises(RuntimeError):
            with Profiler(trace_memory=True):
                pass  # __enter__ should raise before the body
    finally:
        _core.perf_stop()
    assert not tracemalloc.is_tracing(), 'tracemalloc was left running'


def test_profiler_does_not_stop_foreign_tracemalloc_on_failure():
    """If tracemalloc was already tracing (owned elsewhere), a failed Profiler
    must not stop it."""
    import tracemalloc

    tracemalloc.start()
    _core.perf_start(5.0, 64)
    try:
        with pytest.raises(RuntimeError):
            with Profiler(trace_memory=True):
                pass
        assert tracemalloc.is_tracing(), 'foreign tracemalloc was wrongly stopped'
    finally:
        _core.perf_stop()
        if tracemalloc.is_tracing():
            tracemalloc.stop()


# ── repeated start/stop cycles (Arc-not-leak fix stays functional) ────────────


def test_many_profile_cycles_stay_functional():
    """The buffers used to be Box::leak'd on every start; the Arc rewrite frees
    them. Exercise many cycles to confirm start/stop is still correct each time."""
    for _ in range(30):
        with Profiler(interval_ms=1.0) as prof:
            s = 0
            for i in range(20000):
                s += i
        assert prof.result is not None
        assert not _core.perf_running()


# ── full pipeline e2e ─────────────────────────────────────────────────────────

_DEMO = (
    'import time\n'
    'def work():\n'
    '    total = 0\n'
    '    end = time.perf_counter() + 0.2\n'
    '    while time.perf_counter() < end:\n'
    '        total += 1\n'
    '    return total\n'
    'work()\n'
)


@pytest.mark.skipif(sys.platform == 'win32', reason='REMAINDER arg parsing differs; covered on POSIX')
def test_full_run_pipeline(tmp_path):
    """run → HTML + speedscope + JSON + CSV, then diff against itself."""
    src = tmp_path / 'demo.py'
    src.write_text(_DEMO)
    html = tmp_path / 'report.html'
    ss = tmp_path / 'profile.speedscope.json'
    pj = tmp_path / 'profile.json'
    csv = tmp_path / 'functions.csv'

    rc = run_perf_cli(['run', '--out', str(html), '--speedscope', str(ss),
                       '--json', str(pj), '--csv', str(csv), '--interval', '1', str(src)])
    assert rc == 0
    assert 'work' in html.read_text()
    assert json.loads(ss.read_text())['exporter'] == 'rabbitinspect'
    assert csv.read_text().startswith('function,file,line')

    # save_profile_json/load_profile_json round-trip on the same artifact.
    loaded = load_profile_json(str(pj))
    assert loaded.duration_ms > 0

    diff = tmp_path / 'diff.html'
    rc = run_perf_cli(['diff', str(pj), str(pj), '--out', str(diff)])
    assert rc == 0
    assert 'Before' in diff.read_text() and 'After' in diff.read_text()


def test_csv_export_neutralizes_formula_injection(tmp_path):
    """A function/file name starting with =,+,-,@ must be prefixed with ' so a
    spreadsheet treats it as text, not a formula."""
    from rabbitinspect.perf import FunctionStat, ProfileResult, export_functions_csv

    result = ProfileResult(
        duration_ms=10.0, sample_count=1, truncated=False, folded=[], rss=[],
        functions=[
            FunctionStat('=cmd|calc', '@evil.py', 1.0, 2.0, 50.0, 100.0),
            FunctionStat('safe', '/ok.py', 1.0, 2.0, 50.0, 100.0),
        ],
    )
    path = tmp_path / 'fn.csv'
    export_functions_csv(result, str(path))
    lines = path.read_text().splitlines()
    # dangerous cells get a leading ' so spreadsheets treat them as text
    assert lines[1].startswith("'=cmd|calc,")
    assert "'@evil.py" in lines[1]
    # the safe row is untouched (no spurious quote prefix)
    assert lines[2].startswith('safe,')


def _rich_result():
    from rabbitinspect.perf import aggregate, analyze_hotspots

    raw = {
        'frames': ['leaf\t/app/views.py\t12', 'mid\t/app/svc.py\t30',
                   'idle\t/usr/lib/python3.14/selectors.py\t400'],
        'stacks': [[0, 1], [1], [2]], 'ts': [1.0, 2.0, 3.0], 'tids': [1, 1, 1],
        'rss': [[1.0, 5e7]],
        'spans': [{'method': 'GET', 'route': '/x', 'status': 200, 'start_ms': 0.0, 'end_ms': 2.0}],
        'queries': [{'ts_ms': 1.0, 'sql': f'SELECT {i}', 'duration_ms': 1.0, 'origin': '/app/views.py:12'}
                    for i in range(5)],
        'duration_ms': 5.0, 'sample_count': 3, 'truncated': False,
    }
    r = aggregate(raw)
    r.interval_ms = 5.0
    analyze_hotspots(r)
    return r


def test_report_metadata_header():
    from rabbitinspect.perf import render_html

    html = render_html(_rich_result(), title='T')
    assert 'class="sub">' in html
    assert 'Python' in html
    assert 'interval 5 ms' in html  # interval_ms shown when known


def test_report_sticky_nav_links_present_sections():
    from rabbitinspect.perf import render_html

    html = render_html(_rich_result(), app_root='/app')
    assert '<nav class="toc">' in html
    # sections that have content are linked; ones without are not
    assert 'href="#s-db"' in html       # queries present
    assert 'href="#s-req"' in html      # spans present
    assert 'href="#s-flame"' in html    # always
    assert 'href="#top"' in html        # back-to-top


def test_report_nav_omits_absent_sections():
    from rabbitinspect.perf import ProfileResult, render_html

    empty = ProfileResult(duration_ms=1.0, sample_count=0, truncated=False, functions=[], folded=[], rss=[])
    html = render_html(empty)
    assert 'href="#s-db"' not in html      # no queries
    assert 'href="#s-alloc"' not in html   # no allocations
    assert 'href="#s-flame"' in html       # always-present sections still linked


def test_report_sortable_and_app_filter_markup():
    from rabbitinspect.perf import render_html

    html = render_html(_rich_result(), app_root='/app')
    assert 'onclick="rabSort(this)"' in html
    assert 'function rabSort' in html
    # app/dep classification for the client-side "my code" toggle
    assert 'data-app="1"' in html  # /app/views.py
    assert 'data-app="0"' in html  # selectors.py (stdlib)
    assert 'rabAppOnly' in html
    # keyboard shortcuts
    assert 'addEventListener("keydown"' in html


def test_heuristic_app_classification():
    from rabbitinspect.perf import _heuristic_app

    assert _heuristic_app('/home/me/proj/app/views.py') is True
    assert _heuristic_app('/home/me/proj/.venv/lib/python3.14/site-packages/django/x.py') is False
    assert _heuristic_app('<frozen importlib._bootstrap>') is False
    assert _heuristic_app('') is False


def test_save_load_diff_self_zero_delta(tmp_path):
    from rabbitinspect.perf import diff_profiles, profile_script

    src = tmp_path / 'demo.py'
    src.write_text(_DEMO)
    result = profile_script(str(src), interval_ms=1.0)
    pj = tmp_path / 'p.json'
    save_profile_json(result, str(pj))
    loaded = load_profile_json(str(pj))
    deltas = diff_profiles(loaded, loaded)
    assert all(d.delta_ms == 0.0 for d in deltas)
