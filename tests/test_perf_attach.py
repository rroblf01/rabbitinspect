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
        # find the target's stack (the one running target.py)
        target_stack = next((s for s in stacks if any('target.py' in e for e in s)), stacks[0])
        # leaf-first; the sleeping thread's stack is leaf <- middle <- outer <- <module>
        funcs = [entry.split('\t')[0] for entry in target_stack]
        assert funcs[:4] == ['leaf', 'middle', 'outer', '<module>']
        assert all(entry.split('\t')[1].endswith('target.py') for entry in target_stack)
        # line numbers decoded from co_linetable (PEP 626) match the source layout
        lines = [int(entry.split('\t')[2]) for entry in target_stack]
        assert lines[:4] == [3, 5, 7, 8]
    finally:
        child.terminate()
        child.wait(timeout=5)


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
    finally:
        child.terminate()
        child.wait(timeout=5)
