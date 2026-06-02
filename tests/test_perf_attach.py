"""Tests for the out-of-process attach foundation (F4 steps 1-2). Linux-only."""

import ctypes
import os
import platform
import subprocess
import sys
import time

import pytest
from rabbitinspect import _core

pytestmark = pytest.mark.skipif(
    not sys.platform.startswith('linux'),
    reason='remote process introspection is Linux-only',
)


def test_python_info_detects_self():
    info = _core.attach_python_info(os.getpid())
    assert info['is_python'] is True
    assert info['maps_count'] > 0
    assert info['base'] is not None


def test_maps_self_nonempty():
    maps = _core.attach_maps(os.getpid())
    assert len(maps) > 0
    region = maps[0]
    assert set(region) >= {'start', 'end', 'perms', 'offset', 'path'}
    assert region['start'] < region['end']


def test_read_mem_self_known_bytes():
    # A known buffer at a known address — verifies process_vm_readv correctness.
    buf = ctypes.create_string_buffer(b'RABBITINSPECT', 14)
    addr = ctypes.addressof(buf)
    data = _core.attach_read_mem(os.getpid(), addr, 13)
    assert data == b'RABBITINSPECT'


def test_invalid_pid_raises():
    with pytest.raises(OSError):
        _core.attach_maps(2_000_000_000)


def test_interpreter_version_self():
    details = _core.attach_interpreter_info(os.getpid())
    expected = platform.python_version_tuple()  # ('3', '14', '4')
    got = details['version'].split('.')
    assert got[0] == expected[0]
    assert got[1] == expected[1]
    # version_hex high bytes encode major.minor
    assert (details['version_hex'] >> 24) & 0xFF == int(expected[0])
    assert (details['version_hex'] >> 16) & 0xFF == int(expected[1])
    assert details['py_runtime_addr'] > 0


def test_read_remote_rss_self():
    from rabbitinspect.perf import _read_remote_rss

    rss = _read_remote_rss(os.getpid())
    assert rss is not None
    assert rss > 0


def test_attach_to_child_process():
    child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(5)'])
    try:
        time.sleep(0.3)
        info = _core.attach_python_info(child.pid)
        assert info['is_python'] is True

        # Cross-process symbol resolution: read the child's CPython version.
        try:
            details = _core.attach_interpreter_info(child.pid)
        except OSError:
            details = None
        if details is not None:
            assert details['version'].startswith(platform.python_version_tuple()[0] + '.')
            assert details['py_runtime_addr'] > 0

        # Cross-process read: the interpreter image starts with the ELF magic.
        # Skip the assertion if ptrace_scope blocks cross-process reads.
        maps = _core.attach_maps(child.pid)
        region = next(
            (m for m in maps if m['perms'].startswith('r') and m['offset'] == 0 and m['path'].endswith('python3.14')),
            None,
        )
        if region is None:
            region = next((m for m in maps if m['perms'].startswith('r') and m['offset'] == 0 and m['path']), None)
        if region is not None:
            try:
                data = _core.attach_read_mem(child.pid, region['start'], 4)
            except OSError:
                pytest.skip('cross-process read blocked by ptrace_scope')
            assert data[:4] == b'\x7fELF'
    finally:
        child.terminate()
        child.wait(timeout=5)


_TARGET = (
    'import time\n'
    'def leaf():\n'
    '    time.sleep(30)\n'
    'def middle():\n'
    '    leaf()\n'
    'def outer():\n'
    '    middle()\n'
    'outer()\n'
)


def _spawn_target(tmp_path):
    script = tmp_path / 'target.py'
    script.write_text(_TARGET)
    child = subprocess.Popen([sys.executable, str(script)])
    time.sleep(0.6)
    return child


def test_remote_sample_recovers_stack(tmp_path):
    child = _spawn_target(tmp_path)
    try:
        try:
            stacks = _core.attach_sample(child.pid)
        except OSError:
            pytest.skip('remote sampling blocked (ptrace_scope / unsupported version)')
        assert len(stacks) >= 1
        # each thread is {'state': <char>, 'frames': [...]}
        target = next((t for t in stacks if any('target.py' in e for e in t['frames'])), stacks[0])
        frames = target['frames']
        # leaf-first; the sleeping thread's stack is leaf <- middle <- outer <- <module>
        funcs = [entry.split('\t')[0] for entry in frames]
        assert funcs[:4] == ['leaf', 'middle', 'outer', '<module>']
        assert all(entry.split('\t')[1].endswith('target.py') for entry in frames)
        # line numbers decoded from co_linetable (PEP 626) match the source layout
        lines = [int(entry.split('\t')[2]) for entry in frames]
        assert lines[:4] == [3, 5, 7, 8]
        # the target is blocked in time.sleep → off-CPU (not 'R')
        assert target['state'] in {'S', 'D'}
    finally:
        child.terminate()
        child.wait(timeout=5)


def _discover_interpreters():
    """Resolve distinct CPython interpreters available on PATH (3.11–3.15)."""
    import shutil

    seen: dict[tuple[int, int], str] = {}
    candidates = [sys.executable] + [shutil.which(f'python3.{m}') for m in range(11, 16)]
    for exe in candidates:
        if not exe:
            continue
        try:
            out = subprocess.check_output(
                [exe, '-c', 'import sys;print(f"{sys.version_info[0]}.{sys.version_info[1]}")'],
                text=True,
            ).strip()
            major, minor = (int(x) for x in out.split('.'))
        except (OSError, ValueError, subprocess.SubprocessError):
            continue
        seen.setdefault((major, minor), exe)
    return seen


def test_attach_across_python_versions(tmp_path):
    """Validate the version-aware DebugOffsets table on every CPython available.

    3.13+ exposes `_Py_DebugOffsets` (sampling works); 3.11/3.12 do not (a clear
    error). Interpreters not installed on the runner are simply skipped.
    """
    interpreters = _discover_interpreters()
    if len(interpreters) < 2:
        pytest.skip('need a second CPython on PATH to exercise cross-version attach')

    script = tmp_path / 'target.py'
    script.write_text(_TARGET)

    tested = []
    for (major, minor), exe in sorted(interpreters.items()):
        child = subprocess.Popen([exe, str(script)])
        try:
            time.sleep(0.7)
            if minor >= 13:
                try:
                    stacks = _core.attach_sample(child.pid)
                except OSError:
                    pytest.skip(f'sampling blocked for {major}.{minor} (ptrace_scope)')
                target = next(
                    (t for t in stacks if any('target.py' in e for e in t['frames'])), None
                )
                assert target is not None, f'no target stack for {major}.{minor}'
                funcs = [e.split('\t')[0] for e in target['frames']]
                assert funcs[:4] == ['leaf', 'middle', 'outer', '<module>'], f'{major}.{minor}'
                lines = [int(e.split('\t')[2]) for e in target['frames']]
                assert lines[:4] == [3, 5, 7, 8], f'{major}.{minor} lines {lines[:4]}'
            else:
                # No remote-debug offsets before 3.13 → explicit failure.
                with pytest.raises(OSError):
                    _core.attach_sample(child.pid)
            tested.append((major, minor))
        finally:
            child.terminate()
            child.wait(timeout=5)

    assert tested, 'no interpreters exercised'


def test_sample_remote_aggregates(tmp_path):
    from rabbitinspect.perf import sample_remote

    child = _spawn_target(tmp_path)
    try:
        result = sample_remote(child.pid, duration_s=0.5, interval_ms=5.0)
        if result.sample_count == 0:
            pytest.skip('remote sampling blocked (ptrace_scope / unsupported version)')
        names = {f.name for f in result.functions}
        assert {'leaf', 'middle', 'outer'} <= names
        # the sleeping leaf frame dominates self time
        assert result.functions[0].name == 'leaf'
        # the whole run is the target sleeping → classified off-CPU
        assert result.off_cpu_ms > result.on_cpu_ms
        leaf = next(f for f in result.functions if f.name == 'leaf')
        assert leaf.off_cpu_ms > 0.0
        # resident memory sampled from the target's /proc/<pid>/status
        assert result.peak_rss_bytes > 0.0
        assert len(result.rss) >= 1
    finally:
        child.terminate()
        child.wait(timeout=5)


def test_sample_remote_multi(tmp_path):
    from rabbitinspect.perf import sample_remote_multi

    c1 = _spawn_target(tmp_path)
    # second target from a different dir so both run the sleeping stack
    d2 = tmp_path / 'w2'
    d2.mkdir()
    c2 = _spawn_target(d2)
    try:
        result = sample_remote_multi([c1.pid, c2.pid], duration_s=0.6, interval_ms=5.0)
        if result.sample_count == 0:
            pytest.skip('remote sampling blocked (ptrace_scope / unsupported version)')
        names = {f.name for f in result.functions}
        assert {'leaf', 'middle', 'outer'} <= names
        # threads from the two workers are kept distinct
        assert len(set(result.raw.get('tids', []))) >= 2
    finally:
        for c in (c1, c2):
            c.terminate()
            c.wait(timeout=5)
