"""Demo workload for `rabbitinspect perf run examples/perf_demo.py`.

`hot()` dominates CPU time and also contains a static anti-pattern
(`it == None`), so the report's "Hotspots with lint findings" section lights up
— showing the static rules pointed straight at the hottest code.
"""

import time


def hot(items):
    total = 0
    for it in items:
        if it == None:  # noqa: E711  (intentional: triggers RAB002 on a hot line)
            continue
        total += it * it
    return total


def cool():
    return sum(range(100))


def main():
    data = list(range(5000))
    end = time.perf_counter() + 0.6
    while time.perf_counter() < end:
        hot(data)
        cool()


if __name__ == '__main__':
    main()
