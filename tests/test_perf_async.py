"""Tests for asyncio task-awareness sampling (suspended coroutines)."""

import asyncio

from rabbitinspect.perf import (
    AsyncSampler,
    async_task_snapshot,
    profile_asyncio,
)


def test_async_task_snapshot_sees_awaiting_coroutine():
    async def worker():
        await asyncio.sleep(0.3)

    async def main():
        task = asyncio.create_task(worker(), name='w1')
        await asyncio.sleep(0.05)  # let worker park on the sleep
        infos = async_task_snapshot()
        task.cancel()
        return infos

    infos = asyncio.run(main())
    # the awaiting worker coroutine is captured with its stack
    worker_infos = [i for i in infos if any('worker' in e for e in i.stack)]
    assert worker_infos
    assert any(i.name == 'w1' for i in infos)
    assert all(i.state in {'pending', 'done'} for i in infos)


def test_async_task_snapshot_no_loop_returns_empty():
    # called with no running loop → empty, no crash
    assert async_task_snapshot() == []


def test_profile_asyncio_flamegraph_of_await():
    async def worker():
        await asyncio.sleep(0.25)

    async def main():
        await asyncio.gather(worker(), worker(), worker())

    result = profile_asyncio(main, interval_ms=5.0)

    assert result.sample_count > 0
    names = {f.name for f in result.functions}
    assert 'worker' in names
    # the flamegraph / report renders the await hotspots
    html = result.to_html()
    assert 'worker' in html
    assert 'Flamegraph' in html


def test_async_sampler_start_stop_no_tasks():
    # sampling a loop with no extra tasks still produces a valid (empty) result
    async def main():
        sampler = AsyncSampler(asyncio.get_running_loop(), interval_ms=5.0).start()
        await asyncio.sleep(0.02)
        return sampler.stop()

    result = asyncio.run(main())
    assert result.sample_count >= 0
    assert result.duration_ms > 0
