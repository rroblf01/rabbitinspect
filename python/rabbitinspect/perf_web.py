"""Web framework integration for the runtime profiler (F2).

Drop-in WSGI/ASGI middleware that records one request span per request into the
running sampler, so the HTML report can show a request timeline and per-endpoint
latency. Both middlewares are pass-through no-ops when the profiler is not
running, so they are safe to leave installed permanently.

Django (WSGI), in ``wsgi.py``::

    from rabbitinspect.perf_web import WSGIProfilerMiddleware
    application = WSGIProfilerMiddleware(application)

FastAPI / Starlette (ASGI)::

    from rabbitinspect.perf_web import ASGIProfilerMiddleware
    app.add_middleware(ASGIProfilerMiddleware)

Plus :func:`enable_fork_profiling` to keep sampling alive across forked workers
(gunicorn/uvicorn), since OS threads do not survive ``fork()``.
"""

from __future__ import annotations

from rabbitinspect import _core


def _status_code(raw: object) -> int:
    if isinstance(raw, int):
        return raw
    if isinstance(raw, str):
        head = raw.split(' ', 1)[0]
        try:
            return int(head)
        except ValueError:
            return 0
    return 0


class WSGIProfilerMiddleware:
    """Wrap a WSGI application to record a span per request."""

    def __init__(self, app):
        self.app = app

    def __call__(self, environ, start_response):
        if not _core.perf_running():
            return self.app(environ, start_response)

        start = _core.perf_now_ms()
        captured = {'status': 0}

        def _start_response(status, headers, exc_info=None):
            captured['status'] = _status_code(status)
            return start_response(status, headers, exc_info)

        result = self.app(environ, _start_response)
        _core.perf_record_span(
            environ.get('REQUEST_METHOD', 'GET'),
            environ.get('PATH_INFO', '/') or '/',
            captured['status'],
            start,
            _core.perf_now_ms(),
        )
        return result


class ASGIProfilerMiddleware:
    """Wrap an ASGI application to record a span per HTTP request."""

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope.get('type') != 'http' or not _core.perf_running():
            await self.app(scope, receive, send)
            return

        start = _core.perf_now_ms()
        captured = {'status': 0}

        async def _send(message):
            if message.get('type') == 'http.response.start':
                captured['status'] = _status_code(message.get('status', 0))
            await send(message)

        try:
            await self.app(scope, receive, _send)
        finally:
            _core.perf_record_span(
                scope.get('method', 'GET'),
                scope.get('path', '/') or '/',
                captured['status'],
                start,
                _core.perf_now_ms(),
            )


def enable_fork_profiling(interval_ms: float = 5.0, max_depth: int = 256) -> None:
    """Keep the sampler running across forked worker processes.

    Threads do not survive ``fork()``, so a worker inherits a dead sampler. This
    registers an ``after_in_child`` hook that starts a fresh sampler in each new
    worker. Call it once in the parent before workers are forked.
    """
    import os

    def _restart_in_child():
        # The inherited sampler's thread is gone; force a clean restart with a
        # fresh buffer for this worker (only if the parent was profiling).
        if not _core.perf_running():
            return
        try:
            _core.perf_reset()
            _core.perf_start(interval_ms, max_depth)
        except Exception:
            pass

    os.register_at_fork(after_in_child=_restart_in_child)
