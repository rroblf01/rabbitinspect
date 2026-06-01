# ruff: noqa
import os
import time
import sys


def bad_none_compare(x):
    if x == None:
        return True
    return False


def bad_len_is_empty(items):
    return len(items) == 0


def bad_any_list_comp(nums):
    return any([n > 0 for n in nums])


def bad_dict_keys_loop(mapping):
    for k in mapping.keys():
        print(k)


def bad_type_compare(val):
    return type(val) == int


def bad_unnecessary_else(x):
    if x > 0:
        return x
    else:
        return 0


def bad_range_len(items):
    for i in range(len(items)):
        print(items[i])


def bad_str_concat(items):
    s = ''
    for x in items:
        s += str(x)
    return s


def bad_set_list(items):
    return set(list(items))


def bad_mutable_default(x=[]):
    return x


def bad_bare_except():
    try:
        print('hello')
    except:
        pass


def bad_bare_except_pass():
    try:
        print('hello')
    except:
        pass


class BadClassObject(object):
    pass


def bad_keys_in(d, k):
    return k in d.keys()


def bad_format_call(name):
    return 'Hello {}'.format(name)


def bad_function_too_long():
    x0 = 0
    x1 = 1
    x2 = 2
    x3 = 3
    x4 = 4
    x5 = 5
    x6 = 6
    x7 = 7
    x8 = 8
    x9 = 9
    x10 = 10
    x11 = 11
    x12 = 12
    x13 = 13
    x14 = 14
    x15 = 15
    x16 = 16
    x17 = 17
    x18 = 18
    x19 = 19
    x20 = 20
    x21 = 21
    x22 = 22
    x23 = 23
    x24 = 24
    x25 = 25
    x26 = 26
    x27 = 27
    x28 = 28
    x29 = 29
    x30 = 30
    x31 = 31
    x32 = 32


def bad_too_many_params(a, b, c, d, e, f, g):
    return a + b + c + d + e + f + g


def bad_os_system():
    os.system('ls')


def bad_time_time():
    t = time.time()


def bad_redundant_call(obj):
    return obj.call()


def bad_missing_return_hint(x):
    return x


def bad_deep_comprehension(matrix):
    return [[x for y in z for w in z for x in w] for z in matrix]


def bad_long_if_chain(x):
    if x == 1:
        pass
    elif x == 2:
        pass
    elif x == 3:
        pass
    elif x == 4:
        pass


def bad_bool_compare(x):
    if x == True:
        return False
    return True


def bad_bool_return(x):
    if x > 0:
        return True
    else:
        return False


def bad_bool_call(x):
    return bool(x)


def bad_assert_true():
    assert True


def bad_assert_false():
    assert False


def bad_high_cyclomatic(a, b, c, d, e):
    if a > 0:
        for i in range(b):
            if i % 2 == 0:
                print(i)
            elif i % 3 == 0:
                print('three')
            elif i % 5 == 0:
                print('five')
            else:
                print('other')
    elif c > 0:
        while c > 0:
            c -= 1
    else:
        try:
            pass
        except ValueError:
            pass
        except TypeError:
            pass
        except RuntimeError:
            pass
    try:
        with open('/dev/null') as f:
            print(f.read())
    except OSError:
        pass
    assert e > 0
    return a


def bad_high_cognitive(a, b, c, d):
    if a > 0:
        for i in range(b):
            if i % 2 == 0:
                print(i)
            elif i % 3 == 0:
                print('three')
    elif c > 0:
        while c > 0:
            if c == 5:
                break
            c -= 1
    else:
        try:
            pass
        except ValueError:
            pass
        except TypeError:
            pass
    return a


def bad_power_opt(x):
    return x**2


import math


def bad_math_pow(x):
    return math.pow(x, 3)


def bad_map_lambda(items):
    return list(map(lambda x: x + 1, items))


def bad_filter_lambda(items):
    return list(filter(lambda x: x > 0, items))


def bad_list_membership(x):
    return x in [1, 2, 3]


def bad_tuple_membership(x):
    return x not in (1, 2)


from dataclasses import dataclass


@dataclass
class BadDataclass:
    x: int = 0


import re


def bad_re_compile():
    return re.compile('[a-z]')


def bad_readlines():
    f = open('file.txt')
    for line in f.readlines():
        print(line)


from typing import Optional


def bad_optional(x: Optional[int]) -> Optional[str]:
    return None


from typing import Union


bad_union: Union[int, str] = 1


def bad_sorted_list(items):
    return sorted(list(items))


def bad_reversed_tuple(items):
    return reversed(tuple(items))


def bad_dict_get(d, k):
    if k in d:
        return d[k]


def bad_slice_copy(items):
    return items[:]


from typing import List, Dict, Tuple, Set


def bad_list_annotation(x: List[int]) -> None:
    pass


x_bad_dict_annotation: Dict[str, int] = {}


def bad_manual_list(items):
    result = []
    for x in items:
        result.append(x)
    return result


bad_open_expr = open('/dev/null')


def bad_sorted_index0(items):
    return sorted(items)[0]


def bad_sorted_index_neg1(items):
    return sorted(items)[-1]


def bad_not_is_none(x):
    if not x is None:
        return True
    return False


def bad_augmented_assign(x):
    x = x + 1
    return x


def bad_is_true(x):
    if x is True:
        return 1
    return 0


def bad_is_false(x):
    if x is False:
        return 1
    return 0


def bad_range_len_v2(items):
    result = []
    for i in range(len(items)):
        result.append(items[i])
    return result


def bad_setdefault(d, k, v):
    d.setdefault(k, []).append(v)


def bad_type_or(x):
    return type(x) == int or type(x) == str


def bad_if_not_assign(x):
    if not x:
        x = 42
    return x


def bad_unused_loop_var():
    for _unused in range(10):
        print('hello')


def bad_nested_with():
    with open('/dev/null') as f:
        with open('/dev/zero') as g:
            print(f, g)


def bad_startswith_or(s):
    return s.startswith('a') or s.startswith('b')


def bad_return_ternary(x):
    return True if x > 0 else False


def bad_while_true():
    while True:
        print('infinite')


def bad_sorted_sort(items):
    return sorted(items).sort()


from os import *


def bad_redundant_pass():
    """docstring"""
    pass


def bad_is_literal(x):
    return x is 5


class BadInitReturn:
    def __init__(self):
        return 42


def bad_dead_code():
    if True:
        print('always')


def bad_def_in_loop():
    items = [1, 2, 3]
    for x in items:

        def inner():
            return x


list = [1, 2, 3]


def bad_raise_without_from():
    try:
        pass
    except:
        raise ValueError('no from')


def bad_debug_leftover():
    print('debug')
    breakpoint()


def bad_import_in_function():
    import os


x = {'a': 1, 'b': 2, 'a': 3}


def bad_broad_except():
    try:
        pass
    except Exception:
        pass


def bad_unnecessary_pass():
    x = 1
    pass


def bad_missing_param_type(x):
    pass


def bad_class_attr_type():
    class Foo:
        x = 1


x_type = 1


y_any: Any = None


def bad_type_default(x: str = None):
    pass


def bad_redundant_elif(x):
    if x > 0:
        return x
    elif x == 0:
        pass
    elif x < 0:
        pass


def bad_self_comparison(x):
    if x == x:
        pass


def bad_passthrough_gen(items):
    return list(x for x in items)


# TODO: fix this later
y = 1


class bad_class_name:
    pass


def BadFunctionName():
    pass


my_constant = 42


def bad_inconsistent_return(x):
    if x > 0:
        return x
    return


__all__ = ['foo', 42]


eval('print(1)')


pickle.loads(data)


yaml.load(data)


def bad_del_except():
    try:
        pass
    except Exception as e:
        del e


def bad_modify_iter(items):
    for x in items:
        items.remove(x)


asyncio.get_event_loop()


def bad_async_blocking():
    async def inner():
        time.sleep(1)


def bad_magic():
    x = 42


def bad_loop_else(items):
    for x in items:
        if x > 0:
            break
    else:
        print('not found')


result = not x in y
