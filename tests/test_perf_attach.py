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
