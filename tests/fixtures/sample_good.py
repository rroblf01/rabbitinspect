def process_items(data):
    results = [item * 2 for item in data if item is not None]
    return results


def check_value(x):
    if x is None:
        return True
    return False


def is_empty(items):
    return not items


def find_positive(nums):
    return any(n > 0 for n in nums)


def iterate_dict(mapping):
    for k in mapping:
        print(k)


def identify_type(val):
    return isinstance(val, int)


def early_return(x):
    if x > 0:
        return x
    return 0


def bool_compare(x):
    if x:
        return False
    return True


def verbose_bool(x):
    return x > 0


def direct_bool(x):
    result = x
    return result


def simple_assert(x):
    assert x > 0


def simple_function(a):
    if a > 0:
        print('positive')
    return a
