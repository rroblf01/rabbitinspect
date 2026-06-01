# ruff: noqa
import os
import time
import subprocess


def good_none_compare(x) -> bool:
    if x is None:
        return True
    return False


def good_len_is_empty(items) -> bool:
    return not items


def good_any_generator(nums) -> bool:
    return any(n > 0 for n in nums)


def good_dict_keys_loop(mapping) -> None:
    for k in mapping:
        print(k)


def good_type_compare(val) -> bool:
    return isinstance(val, int)


def good_no_unnecessary_else(x) -> int:
    if x > 0:
        return x
    return 0


def good_enumerate(items) -> None:
    for i, item in enumerate(items):
        print(i, item)


def good_join(items) -> str:
    return ''.join(str(x) for x in items)


def good_direct_set(items) -> set:
    return set(items)


def good_none_default(x=None) -> list:
    if x is None:
        x = []
    return x


def good_typed_except() -> None:
    try:
        print('hello')
    except ValueError:
        pass


def good_typed_except_action() -> None:
    try:
        print('hello')
    except ValueError:
        print('error')


class GoodClass:
    pass


def good_keys_in(d, k) -> bool:
    return k in d


def good_fstring(name) -> str:
    return f'Hello {name}'


def good_short_function(x) -> int:
    y = x + 1
    return y


def good_few_params(a, b, c) -> int:
    return a + b + c


def good_subprocess() -> None:
    subprocess.run(['ls'])


def good_perf_counter() -> float:
    return time.perf_counter()


def good_normal_call(obj) -> str:
    return obj.method()


def good_with_return_hint(x) -> int:
    return x


def good_simple_comprehension(items) -> list:
    return [x for x in items]


def good_short_if_chain(x) -> int:
    if x == 1:
        return 1
    elif x == 2:
        return 2
    elif x == 3:
        return 3
    return 0


def good_bool_compare(x) -> bool:
    if x:
        return False
    return True


def good_bool_return(x) -> bool:
    return x > 0


def good_direct_bool(x) -> bool:
    return x


def good_assert_expr(x) -> None:
    assert x > 0


def good_simple_function(a) -> None:
    if a > 0:
        print('positive')


def good_power_opt(x) -> int:
    return x * x


import math


def good_pow_literal(x) -> int:
    return math.pow(x, 5)


def good_map_comp(items) -> list:
    return [x + 1 for x in items]


def good_filter_comp(items) -> list:
    return [x for x in items if x > 0]


def good_set_membership(x) -> bool:
    return x in {1, 2, 3}


from dataclasses import dataclass


@dataclass(slots=True)
class GoodDataclass:
    x: int = 0


import re

pattern = re.compile('[a-z]')


def good_use_pattern(s) -> bool:
    return pattern.match(s) is not None


def good_readlines() -> None:
    with open('file.txt') as f:
        for line in f:
            print(line)


def good_int_annotation(x: int) -> None:
    pass


def good_sorted_direct(items) -> list:
    return sorted(items)


def good_dict_get(d, k) -> None:
    d.get(k)


def good_slice_copy(items) -> list:
    return items.copy()


from typing import List, Dict


def good_list_annotation(x: list[int]) -> None:
    pass


def good_comprehension(items) -> list:
    return [x for x in items]


def good_with_open() -> None:
    with open('file.txt') as f:
        print(f.read())


def good_sorted_index0(items) -> int:
    return min(items)


def good_sorted_index_neg1(items) -> int:
    return max(items)


def good_not_is_none(x) -> bool:
    if x is not None:
        return True
    return False


def good_augmented_assign(x) -> int:
    x += 1
    return x


def good_is_true(x) -> int:
    if x:
        return 1
    return 0


def good_is_false(x) -> int:
    if not x:
        return 1
    return 0


def good_range_len(items) -> list:
    return [item for item in items]


def good_setdefault(d, k, v) -> None:
    d[k].append(v)


def good_isinstance(x) -> bool:
    return isinstance(x, (int, str))


def good_or_assign(x) -> int:
    x = x or 42
    return x


def good_used_loop_var() -> None:
    for i in range(10):
        print(i)


def good_single_with() -> None:
    with open('/dev/null') as f:
        print(f)


def good_startswith_or(s) -> bool:
    return s.startswith(('a', 'b'))


def good_return_cond(x) -> bool:
    return x > 0


def good_while_with_break() -> None:
    while True:
        break


def good_direct_sort(items) -> None:
    items.sort()


from os import path


def good_no_redundant_pass() -> None:
    """docstring"""
    print('body')


def good_is_literal(x) -> bool:
    return x == 5


class GoodInitReturn:
    def __init__(self) -> None:
        pass


def good_no_dead_code(x) -> None:
    if x > 0:
        print('maybe')


def good_def_outside_loop() -> None:
    def inner() -> None:
        pass

    for x in range(10):
        inner()


my_list = [1, 2, 3]


def good_raise_with_from() -> None:
    try:
        pass
    except Exception as e:
        raise ValueError('with from') from e
