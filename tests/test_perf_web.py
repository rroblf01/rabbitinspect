"""Tests for web-framework profiler integration (F2)."""

import asyncio
import os
import sys

import pytest
from rabbitinspect import _core
from rabbitinspect.perf import Span, aggregate, render_html
from rabbitinspect.perf_web import (
    ASGIProfilerMiddleware,
    WSGIProfilerMiddleware,
    enable_fork_profiling,
)


@pytest.fixture(autouse=True)
def _ensure_sampler_stopped():
    yield
    if _core.perf_running():
        _core.perf_stop()


# ── WSGI ─────────────────────────────────────────────────────────────────────


def _wsgi_app(environ, start_response):
    start_response('200 OK', [('Content-Type', 'text/plain')])
    return [b'ok']


def test_wsgi_records_span():
    mw = WSGIProfilerMiddleware(_wsgi_app)
    _core.perf_start(5.0, 64)
    body = b''.join(mw({'REQUEST_METHOD': 'GET', 'PATH_INFO': '/items'}, lambda s, h, e=None: None))
    raw = _core.perf_stop()
    assert body == b'ok'
    assert len(raw['spans']) == 1
    span = raw['spans'][0]
    assert span['method'] == 'GET'
    assert span['route'] == '/items'
    assert span['status'] == 200


def test_wsgi_passthrough_when_not_running():
    mw = WSGIProfilerMiddleware(_wsgi_app)
    assert not _core.perf_running()
    body = b''.join(mw({'REQUEST_METHOD': 'GET', 'PATH_INFO': '/x'}, lambda s, h, e=None: None))
    assert body == b'ok'  # works fine without an active profiler


def test_wsgi_captures_error_status():
    def boom_app(environ, start_response):
        start_response('500 Internal Server Error', [])
        return [b'']

    mw = WSGIProfilerMiddleware(boom_app)
    _core.perf_start(5.0, 64)
    list(mw({'REQUEST_METHOD': 'POST', 'PATH_INFO': '/crash'}, lambda s, h, e=None: None))
    raw = _core.perf_stop()
    assert raw['spans'][0]['status'] == 500


# ── ASGI ─────────────────────────────────────────────────────────────────────


def test_asgi_records_span():
    async def asgi_app(scope, receive, send):
        await send({'type': 'http.response.start', 'status': 201, 'headers': []})
        await send({'type': 'http.response.body', 'body': b'created'})

    sent: list = []

    async def send(message):
        sent.append(message)

    async def receive():
        return {'type': 'http.request'}

    async def run():
        mw = ASGIProfilerMiddleware(asgi_app)
        await mw({'type': 'http', 'method': 'POST', 'path': '/submit'}, receive, send)

    _core.perf_start(5.0, 64)
    asyncio.run(run())
    raw = _core.perf_stop()
    assert len(raw['spans']) == 1
    assert raw['spans'][0]['method'] == 'POST'
    assert raw['spans'][0]['route'] == '/submit'
    assert raw['spans'][0]['status'] == 201
    assert any(m['type'] == 'http.response.body' for m in sent)


def test_asgi_passthrough_non_http():
    called = {'n': 0}

    async def asgi_app(scope, receive, send):
        called['n'] += 1

    async def run():
        mw = ASGIProfilerMiddleware(asgi_app)
        await mw({'type': 'lifespan'}, None, None)

    _core.perf_start(5.0, 64)
    asyncio.run(run())
    raw = _core.perf_stop()
    assert called['n'] == 1
    assert len(raw['spans']) == 0  # non-http scope is not recorded


# ── span primitives ──────────────────────────────────────────────────────────


def test_now_ms_and_record_noop_when_not_running():
    assert not _core.perf_running()
    assert _core.perf_now_ms() == -1.0
    # must not raise even though nothing is recording
    _core.perf_record_span('GET', '/x', 200, 0.0, 1.0)


def test_now_ms_running():
    _core.perf_start(5.0, 64)
    try:
        assert _core.perf_now_ms() >= 0.0
    finally:
        _core.perf_stop()


# ── endpoint aggregation ─────────────────────────────────────────────────────


def test_aggregate_endpoints_percentiles():
    durations = [10, 20, 30, 40, 100]
    spans = [{'method': 'GET', 'route': '/a', 'status': 200, 'start_ms': 0.0, 'end_ms': float(d)} for d in durations]
    spans.append({'method': 'GET', 'route': '/a', 'status': 500, 'start_ms': 0.0, 'end_ms': 5.0})
    raw = {
        'frames': [],
        'stacks': [],
        'ts': [],
        'tids': [],
        'rss': [],
        'spans': spans,
        'duration_ms': 100.0,
        'sample_count': 0,
        'truncated': False,
    }
    result = aggregate(raw)
    assert len(result.endpoints) == 1
    ep = result.endpoints[0]
    assert ep.method == 'GET' and ep.route == '/a'
    assert ep.count == 6
    assert ep.max_ms == 100.0
    assert ep.errors == 1  # the 5xx
    # p50 of [5,10,20,30,40,100] -> between 20 and 30
    assert 20.0 <= ep.p50_ms <= 30.0


def test_render_html_requests_section():
    result = aggregate(
        {
            'frames': [],
            'stacks': [],
            'ts': [],
            'tids': [],
            'rss': [],
            'spans': [{'method': 'GET', 'route': '/health', 'status': 200, 'start_ms': 1.0, 'end_ms': 4.0}],
            'duration_ms': 10.0,
            'sample_count': 0,
            'truncated': False,
        }
    )
    html = render_html(result)
    assert 'Requests' in html
    assert '/health' in html
    assert '<rect' in html  # gantt bar


def test_no_requests_section_without_spans():
    from rabbitinspect.perf import ProfileResult

    result = ProfileResult(duration_ms=1.0, sample_count=0, truncated=False, functions=[], folded=[], rss=[])
    assert 'Requests' not in render_html(result)


def test_span_duration():
    assert Span('GET', '/', 200, 2.0, 5.5).duration_ms == 3.5


# ── fork handling ────────────────────────────────────────────────────────────


@pytest.mark.skipif(not hasattr(os, 'fork'), reason='fork is not available on this platform')
@pytest.mark.skipif(not sys.platform.startswith('linux'), reason='fork+RSS test is Linux-only')
def test_fork_restarts_sampler_in_child():
    enable_fork_profiling(interval_ms=2.0, max_depth=64)
    _core.perf_start(5.0, 64)
    try:
        pid = os.fork()
        if pid == 0:
            # Child: the at_fork hook should have restarted a fresh sampler.
            code = 0
            try:
                assert _core.perf_running()
                # do a little work, then stop cleanly (joins the *new* thread)
                s = 0
                for i in range(200000):
                    s += i
                raw = _core.perf_stop()
                assert 'spans' in raw
            except Exception:
                code = 1
            os._exit(code)
        else:
            _, status = os.waitpid(pid, 0)
            assert os.WIFEXITED(status)
            assert os.WEXITSTATUS(status) == 0
    finally:
        if _core.perf_running():
            _core.perf_stop()
