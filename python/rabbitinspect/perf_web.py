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


def _dump_slow(method: str, route: str, duration_ms: float, dump_dir: str) -> None:
    """Write an HTML report for a slow request (snapshot, no stop)."""
    import os
    import re
    import time

    from rabbitinspect.perf import aggregate, analyze_hotspots

    try:
        result = aggregate(_core.perf_snapshot())
        analyze_hotspots(result)
        os.makedirs(dump_dir, exist_ok=True)
        slug = re.sub(r'[^A-Za-z0-9]+', '_', f'{method}{route}').strip('_') or 'req'
        path = os.path.join(dump_dir, f'slow-{slug}-{int(duration_ms)}ms-{int(time.time())}.html')
        with open(path, 'w', encoding='utf-8') as f:
            f.write(result.to_html())
    except Exception:
        pass


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
    """Wrap a WSGI application to record a span per request.

    ``slow_request_ms`` (with ``dump_dir``): when a request exceeds the threshold,
    write an HTML report for it (snapshot, profiler keeps running)."""

    def __init__(self, app, slow_request_ms: float | None = None, dump_dir: str = 'rabbitinspect-slow'):
        self.app = app
        self.slow_request_ms = slow_request_ms
        self.dump_dir = dump_dir

    def __call__(self, environ, start_response):
        if not _core.perf_running():
            return self.app(environ, start_response)

        start = _core.perf_now_ms()
        captured = {'status': 0}

        def _start_response(status, headers, exc_info=None):
            captured['status'] = _status_code(status)
            return start_response(status, headers, exc_info)

        result = self.app(environ, _start_response)
        end = _core.perf_now_ms()
        method = environ.get('REQUEST_METHOD', 'GET')
        route = environ.get('PATH_INFO', '/') or '/'
        _core.perf_record_span(method, route, captured['status'], start, end)
        if self.slow_request_ms is not None and (end - start) >= self.slow_request_ms:
            _dump_slow(method, route, end - start, self.dump_dir)
        return result


class ASGIProfilerMiddleware:
    """Wrap an ASGI application to record a span per HTTP request.

    ``slow_request_ms`` (with ``dump_dir``): when a request exceeds the threshold,
    write an HTML report for it (snapshot, profiler keeps running)."""

    def __init__(self, app, slow_request_ms: float | None = None, dump_dir: str = 'rabbitinspect-slow'):
        self.app = app
        self.slow_request_ms = slow_request_ms
        self.dump_dir = dump_dir

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
            end = _core.perf_now_ms()
            method = scope.get('method', 'GET')
            route = scope.get('path', '/') or '/'
            _core.perf_record_span(method, route, captured['status'], start, end)
            if self.slow_request_ms is not None and (end - start) >= self.slow_request_ms:
                _dump_slow(method, route, end - start, self.dump_dir)


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
