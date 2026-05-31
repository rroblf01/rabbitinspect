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
    return "".join(str(x) for x in items)


def good_direct_set(items) -> set:
    return set(items)


def good_none_default(x=None) -> list:
    if x is None:
        x = []
    return x


def good_typed_except() -> None:
    try:
        print("hello")
    except ValueError:
        pass


def good_typed_except_action() -> None:
    try:
        print("hello")
    except ValueError:
        print("error")


class GoodClass:
    pass


def good_keys_in(d, k) -> bool:
    return k in d


def good_fstring(name) -> str:
    return f"Hello {name}"


def good_short_function(x) -> int:
    y = x + 1
    return y


def good_few_params(a, b, c) -> int:
    return a + b + c


def good_subprocess() -> None:
    subprocess.run(["ls"])


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
        print("positive")
