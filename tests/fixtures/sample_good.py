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
