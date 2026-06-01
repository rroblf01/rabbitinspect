"""Speedscope export + end-to-end profiler integration tests."""

import pytest
from rabbitinspect import _core
from rabbitinspect.perf import aggregate, profile_script, to_speedscope


@pytest.fixture(autouse=True)
def _ensure_sampler_stopped():
    yield
    if _core.perf_running():
        _core.perf_stop()


def _raw_with_stacks():
    return {
        'frames': ['leaf\tapp.py\t10', 'mid\tapp.py\t5', 'root\tapp.py\t1'],
        'stacks': [[0, 1, 2], [1, 2]],  # leaf-first
        'ts': [1.0, 2.0],
        'tids': [1, 1],
        'rss': [],
        'spans': [],
        'queries': [],
        'duration_ms': 20.0,
        'sample_count': 2,
        'truncated': False,
    }


# ── speedscope export ────────────────────────────────────────────────────────


def test_speedscope_structure():
    ss = to_speedscope(aggregate(_raw_with_stacks()))
    assert ss['$schema'] == 'https://www.speedscope.app/file-format-schema.json'
    assert ss['exporter'] == 'rabbitinspect'
    prof = ss['profiles'][0]
    assert prof['type'] == 'sampled'
    assert prof['unit'] == 'milliseconds'
    assert prof['endValue'] == 20.0
    assert len(prof['samples']) == 2
    assert len(prof['weights']) == 2
    # samples must be root-first (our stacks are leaf-first -> reversed)
    assert prof['samples'][0] == [2, 1, 0]


def test_speedscope_frames_mapped():
    ss = to_speedscope(aggregate(_raw_with_stacks()))
    frames = ss['shared']['frames']
    assert frames[0] == {'name': 'leaf', 'file': 'app.py', 'line': 10}


def test_speedscope_empty():
    ss = to_speedscope(
        aggregate(
            {
                'frames': [],
                'stacks': [],
                'ts': [],
                'tids': [],
                'rss': [],
                'spans': [],
                'queries': [],
                'duration_ms': 0.0,
                'sample_count': 0,
                'truncated': False,
            }
        )
    )
    assert ss['profiles'][0]['samples'] == []
    assert ss['shared']['frames'] == []


# ── end-to-end ───────────────────────────────────────────────────────────────

_DEMO = (
    'import time\n'
    'def hot(items):\n'
    '    total = 0\n'
    '    for it in items:\n'
    '        if it == None:\n'  # RAB002 on a hot line
    '            continue\n'
    '        total += it * it\n'
    '    return total\n'
    'data = list(range(5000))\n'
    'end = time.perf_counter() + 0.4\n'
    'while time.perf_counter() < end:\n'
    '    hot(data)\n'
)


def test_end_to_end_profile_and_report(tmp_path):
    script = tmp_path / 'demo.py'
    script.write_text(_DEMO)

    result = profile_script(str(script), interval_ms=1.0)

    # sampler collected data and stopped cleanly
    assert result.sample_count > 0
    assert not _core.perf_running()

    # hottest function is `hot`
    assert result.functions[0].name == 'hot'

    # hotspot <-> lint cross-reference fired (profile_script runs analyze_hotspots)
    codes = {f['code'] for h in result.hotspot_lints for f in h.findings}
    assert 'RAB002' in codes

    # HTML report has the key sections
    html = result.to_html()
    assert 'Top functions' in html
    assert 'Hotspots with lint findings' in html

    # speedscope export is non-empty and well-formed
    ss = to_speedscope(result)
    assert ss['profiles'][0]['samples']
    assert ss['shared']['frames']
