"""Tests for SQL/N+1 analysis and hotspot-lint cross-reference (F3)."""

import pytest

from rabbitinspect import _core
from rabbitinspect.perf import (
    FunctionStat,
    ProfileResult,
    aggregate,
    analyze_hotspots,
    normalize_sql,
    render_html,
)
from rabbitinspect.perf_db import query_timer, record_query


@pytest.fixture(autouse=True)
def _ensure_sampler_stopped():
    yield
    if _core.perf_running():
        _core.perf_stop()


def _raw(spans=None, queries=None):
    return {
        'frames': [],
        'stacks': [],
        'ts': [],
        'tids': [],
        'rss': [],
        'spans': spans or [],
        'queries': queries or [],
        'duration_ms': 100.0,
        'sample_count': 0,
        'truncated': False,
    }


# ── SQL normalization ────────────────────────────────────────────────────────


def test_normalize_strips_literals():
    assert normalize_sql('SELECT * FROM t WHERE id = 5') == 'SELECT * FROM t WHERE id = ?'
    assert normalize_sql("SELECT * FROM t WHERE name = 'bob'") == 'SELECT * FROM t WHERE name = ?'


def test_normalize_collapses_in_list():
    assert normalize_sql('SELECT * FROM t WHERE id IN (1, 2, 3)') == 'SELECT * FROM t WHERE id IN (?)'


def test_normalize_collapses_whitespace():
    assert normalize_sql('SELECT\n  *\nFROM   t') == 'SELECT * FROM t'


# ── query recording ──────────────────────────────────────────────────────────


def test_record_query_roundtrip():
    _core.perf_start(5.0, 64)
    record_query('SELECT 1', 2.5)
    raw = _core.perf_stop()
    assert len(raw['queries']) == 1
    assert raw['queries'][0]['sql'] == 'SELECT 1'
    assert raw['queries'][0]['duration_ms'] == pytest.approx(2.5)


def test_record_query_noop_when_not_running():
    assert not _core.perf_running()
    record_query('SELECT 1', 1.0)  # must not raise


def test_query_timer_records():
    _core.perf_start(5.0, 64)
    with query_timer('SELECT slow'):
        pass
    raw = _core.perf_stop()
    assert len(raw['queries']) == 1
    assert raw['queries'][0]['sql'] == 'SELECT slow'
    assert raw['queries'][0]['duration_ms'] >= 0.0


# ── N+1 detection ────────────────────────────────────────────────────────────


def test_n_plus_one_detected():
    spans = [{'method': 'GET', 'route': '/list', 'status': 200, 'start_ms': 0.0, 'end_ms': 100.0}]
    queries = [
        {'ts_ms': float(i), 'sql': f'SELECT * FROM item WHERE id = {i}', 'duration_ms': 1.0} for i in range(5)
    ]
    result = aggregate(_raw(spans, queries))
    assert len(result.n_plus_one) == 1
    npo = result.n_plus_one[0]
    assert npo.route == '/list'
    assert npo.max_count == 5
    assert npo.normalized_sql == 'SELECT * FROM item WHERE id = ?'
    assert npo.requests_affected == 1


def test_distinct_queries_not_flagged():
    spans = [{'method': 'GET', 'route': '/ok', 'status': 200, 'start_ms': 0.0, 'end_ms': 100.0}]
    queries = [
        {'ts_ms': 1.0, 'sql': 'SELECT * FROM a', 'duration_ms': 1.0},
        {'ts_ms': 2.0, 'sql': 'SELECT * FROM b', 'duration_ms': 1.0},
    ]
    result = aggregate(_raw(spans, queries))
    assert result.n_plus_one == []
    assert result.query_count == 2


def test_queries_without_request_bucketed():
    queries = [
        {'ts_ms': 1.0, 'sql': 'SELECT 1', 'duration_ms': 1.0},
        {'ts_ms': 2.0, 'sql': 'SELECT 2', 'duration_ms': 1.0},
    ]
    result = aggregate(_raw(spans=[], queries=queries))
    # Same shape "SELECT ?" repeated twice with no covering request -> one offender.
    assert len(result.n_plus_one) == 1
    assert result.n_plus_one[0].route == '(no request)'
    assert result.n_plus_one[0].max_count == 2


def test_query_total_ms():
    queries = [{'ts_ms': 1.0, 'sql': 'X', 'duration_ms': 3.0}, {'ts_ms': 2.0, 'sql': 'Y', 'duration_ms': 4.0}]
    result = aggregate(_raw(queries=queries))
    assert result.query_total_ms == pytest.approx(7.0)


# ── HTML database section ────────────────────────────────────────────────────


def test_html_database_section():
    spans = [{'method': 'GET', 'route': '/list', 'status': 200, 'start_ms': 0.0, 'end_ms': 100.0}]
    queries = [{'ts_ms': float(i), 'sql': f'SELECT x WHERE id={i}', 'duration_ms': 1.0} for i in range(4)]
    html = render_html(aggregate(_raw(spans, queries)))
    assert 'Database' in html
    assert 'N+1' in html
    assert 'Slowest queries' in html


def test_no_database_section_without_queries():
    html = render_html(aggregate(_raw()))
    assert 'Database' not in html


# ── hotspot ↔ lint cross-reference ───────────────────────────────────────────


def _result_for_file(path: str, func: str = 'hot') -> ProfileResult:
    return ProfileResult(
        duration_ms=10.0,
        sample_count=1,
        truncated=False,
        functions=[FunctionStat(func, path, 9.0, 9.0, 90.0, 90.0)],
        folded=[],
        rss=[],
    )


def test_hotspot_lint_cross_reference(tmp_path):
    src = tmp_path / 'hot.py'
    src.write_text('def hot(x):\n    return x == None\n')  # RAB002 on line 2
    result = _result_for_file(str(src))
    hotspots = analyze_hotspots(result)
    assert len(hotspots) == 1
    assert hotspots[0].function == 'hot'
    codes = [f['code'] for f in hotspots[0].findings]
    assert 'RAB002' in codes


def test_hotspot_only_matches_findings_inside_function(tmp_path):
    src = tmp_path / 'mixed.py'
    # RAB002 is in `other`, not in `hot`; hot itself is clean.
    src.write_text(
        'def hot(x):\n'
        '    return x + 1\n'
        '\n'
        'def other(y):\n'
        '    return y == None\n'
    )
    result = _result_for_file(str(src), func='hot')
    hotspots = analyze_hotspots(result)
    assert hotspots == []  # the finding belongs to `other`, not the hot function


def test_hotspot_skips_missing_files():
    result = _result_for_file('/nonexistent/path/to/file.py')
    assert analyze_hotspots(result) == []


def test_hotspot_section_in_html(tmp_path):
    src = tmp_path / 'h.py'
    src.write_text('def hot(x):\n    return x == None\n')
    result = _result_for_file(str(src))
    analyze_hotspots(result)
    html = render_html(result)
    assert 'Hotspots with lint findings' in html
    assert 'RAB002' in html
