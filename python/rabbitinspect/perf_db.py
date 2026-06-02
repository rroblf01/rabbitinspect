"""Database query instrumentation for the runtime profiler (F3).

Records each SQL statement and its duration into the running sampler so the
report can surface slow queries and N+1 patterns per endpoint. All helpers are
no-ops when the profiler is not running.

Generic::

    from rabbitinspect.perf_db import query_timer
    with query_timer('SELECT ...'):
        cursor.execute('SELECT ...')

SQLAlchemy::

    from rabbitinspect.perf_db import instrument_sqlalchemy
    instrument_sqlalchemy(engine)

Django (once at startup, e.g. in AppConfig.ready)::

    from rabbitinspect.perf_db import instrument_django
    instrument_django()
"""

from __future__ import annotations

import os
import sys
import time
from contextlib import contextmanager

from rabbitinspect import _core

_SKIP_DIRS = ('site-packages', 'dist-packages', '.venv', 'venv')
_PKG_DIR = os.path.dirname(os.path.abspath(__file__))  # the rabbitinspect package itself


def _query_origin() -> str:
    """Walk the stack for the first application frame that issued the query.

    Skips this package, the ORM/driver (site-packages / venv), and synthetic
    frames, so the report can point at the app line that triggered the query.
    """
    frame = sys._getframe(1)
    while frame is not None:
        fn = frame.f_code.co_filename
        parts = fn.split(os.sep)
        if (
            not fn.startswith('<')
            and not os.path.abspath(fn).startswith(_PKG_DIR)
            and not any(d in parts for d in _SKIP_DIRS)
        ):
            return f'{fn}:{frame.f_lineno}'
        frame = frame.f_back
    return ''


def record_query(sql: str, duration_ms: float, origin: str | None = None) -> None:
    """Record a query directly (no-op if the profiler is not running)."""
    if not _core.perf_running():
        return
    if origin is None:
        origin = _query_origin()
    _core.perf_record_query(str(sql), float(duration_ms), origin)


@contextmanager
def query_timer(sql: str):
    """Time a block and record it as a query."""
    start = time.perf_counter()
    try:
        yield
    finally:
        record_query(sql, (time.perf_counter() - start) * 1000.0)


def instrument_sqlalchemy(engine) -> None:
    """Attach cursor-execute listeners to a SQLAlchemy ``Engine``."""
    from sqlalchemy import event  # ty: ignore[unresolved-import]

    @event.listens_for(engine, 'before_cursor_execute')
    def _before(conn, cursor, statement, parameters, context, executemany):  # noqa: ANN001
        context._rab_start = time.perf_counter()

    @event.listens_for(engine, 'after_cursor_execute')
    def _after(conn, cursor, statement, parameters, context, executemany):  # noqa: ANN001
        start = getattr(context, '_rab_start', None)
        if start is not None and _core.perf_running():
            record_query(statement, (time.perf_counter() - start) * 1000.0)


def _django_execute_wrapper(execute, sql, params, many, context):  # noqa: ANN001
    start = time.perf_counter()
    try:
        return execute(sql, params, many)
    finally:
        if _core.perf_running():
            record_query(sql, (time.perf_counter() - start) * 1000.0)


def instrument_django() -> None:
    """Record queries on all current and future Django DB connections."""
    from django.db import connections  # ty: ignore[unresolved-import]
    from django.db.backends.signals import connection_created  # ty: ignore[unresolved-import]

    def _install(connection, **kwargs):  # noqa: ANN001
        if _django_execute_wrapper not in connection.execute_wrappers:
            connection.execute_wrappers.append(_django_execute_wrapper)

    connection_created.connect(_install)
    for connection in connections.all():
        _install(connection)
