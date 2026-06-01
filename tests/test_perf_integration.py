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
    assert 'functions by self time' in html
    assert 'Hotspots with lint findings' in html

    # speedscope export is non-empty and well-formed
    ss = to_speedscope(result)
    assert ss['profiles'][0]['samples']
    assert ss['shared']['frames']


_MEM_DEMO = """
def make_big():
    data = []
    for _ in range(50):
        data.append(bytearray(10000))
    return data

# keep the allocations alive at module scope so they're still live at snapshot
BLOBS = make_big()

def main():
    total = 0
    for _ in range(200000):
        total += 1
    return total

main()
"""


def test_profile_with_memory(tmp_path):
    from rabbitinspect.perf import Profiler

    script = tmp_path / 'memdemo.py'
    script.write_text(_MEM_DEMO)

    import runpy

    with Profiler(interval_ms=1.0, trace_memory=True) as prof:
        runpy.run_path(str(script), run_name='__main__')
    result = prof.result

    assert result.mem_allocations, 'tracemalloc produced no allocations'
    # the bytearray allocations should attribute to make_big (or its source line)
    funcs = {a.function for a in result.mem_allocations}
    assert 'make_big' in funcs
    big = next(a for a in result.mem_allocations if a.function == 'make_big')
    assert big.size_bytes > 100_000  # 50 × 10 KB

    html = result.to_html()
    assert 'Top allocations by size' in html
    assert 'make_big' in html


def test_no_memory_section_without_tracing(tmp_path):
    script = tmp_path / 'demo.py'
    script.write_text(_DEMO)
    result = profile_script(str(script), interval_ms=1.0)
    assert result.mem_allocations == []
    assert 'Top allocations by size' not in result.to_html()


def test_profile_json_roundtrip_and_diff(tmp_path):
    from rabbitinspect.perf import diff_profiles, load_profile_json, save_profile_json

    script = tmp_path / 'demo.py'
    script.write_text(_DEMO)
    result = profile_script(str(script), interval_ms=1.0)

    path = tmp_path / 'profile.json'
    save_profile_json(result, str(path))
    loaded = load_profile_json(str(path))

    assert loaded.duration_ms == pytest.approx(result.duration_ms)
    assert loaded.sample_count == result.sample_count
    assert {f.name for f in loaded.functions} == {f.name for f in result.functions}
    # diffing a profile against itself yields no net change
    deltas = diff_profiles(loaded, loaded)
    assert all(d.delta_ms == 0.0 for d in deltas)


def test_perf_diff_cli(tmp_path):
    from rabbitinspect.perf import run_perf_cli

    script = tmp_path / 'demo.py'
    script.write_text(_DEMO)
    before = tmp_path / 'a.json'
    after = tmp_path / 'b.json'
    rc = run_perf_cli(['run', '--out', str(tmp_path / 'a.html'),
                       '--json', str(before), '--interval', '1', str(script)])
    assert rc == 0
    rc = run_perf_cli(['run', '--out', str(tmp_path / 'b.html'),
                       '--json', str(after), '--interval', '1', str(script)])
    assert rc == 0

    diff_html = tmp_path / 'diff.html'
    rc = run_perf_cli(['diff', str(before), str(after), '--out', str(diff_html)])
    assert rc == 0
    text = diff_html.read_text()
    assert 'Before' in text and 'After' in text
    assert 'hot' in text
