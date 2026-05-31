import os
import time


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
    s = ""
    for x in items:
        s += str(x)
    return s


def bad_set_list(items):
    return set(list(items))


def bad_mutable_default(x=[]):
    return x


def bad_bare_except():
    try:
        print("hello")
    except:
        pass


def bad_bare_except_pass():
    try:
        print("hello")
    except:
        pass


class BadClassObject(object):
    pass


def bad_keys_in(d, k):
    return k in d.keys()


def bad_format_call(name):
    return "Hello {}".format(name)


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
    os.system("ls")


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
                print("three")
            elif i % 5 == 0:
                print("five")
            else:
                print("other")
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
        with open("/dev/null") as f:
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
                print("three")
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
