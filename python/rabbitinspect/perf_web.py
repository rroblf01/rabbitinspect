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

    Threads do not survive ``fork()``, so a worker would inherit a dead sampler.
    We also stop the sampler *before* each fork so the fork happens while the
    process is single-threaded — this sidesteps the "fork() in a multi-threaded
    process may deadlock" hazard (a sampler thread holding a lock at the fork
    instant could leave the child stuck) and the matching DeprecationWarning.
    The sampler is then restarted in both the parent and each child. Call once in
    the parent before workers are forked.

    Note: stopping around the fork drops the parent's samples accumulated so far;
    in the usual case (gunicorn/uvicorn fork workers at startup) there is nothing
    to lose yet.
    """
    import os

    was_running = [False]

    def _before():
        # Pause the sampler so the fork is single-threaded (join the thread).
        was_running[0] = _core.perf_running()
        if was_running[0]:
            try:
                _core.perf_stop()
            except Exception:
                pass

    def _after_parent():
        if was_running[0]:
            try:
                _core.perf_start(interval_ms, max_depth)
            except Exception:
                pass

    def _after_child():
        # Fresh sampler + buffer for this worker (only if the parent was profiling).
        if was_running[0]:
            try:
                _core.perf_reset()
                _core.perf_start(interval_ms, max_depth)
            except Exception:
                pass

    os.register_at_fork(before=_before, after_in_parent=_after_parent, after_in_child=_after_child)
