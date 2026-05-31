def process_items(data):
    unused_var = 42
    results = []
    for item in data:
        if item is not None:
            results.append(item * 2)
    return results


def check_value(x):
    if x == None:
        return True
    return False


def is_empty(items):
    return len(items) == 0


def find_positive(nums):
    return any([n > 0 for n in nums])


def iterate_dict(mapping):
    for k in mapping.keys():
        print(k)


def identify_type(val):
    return type(val) == int


def early_return(x):
    if x > 0:
        return x
    else:
        return 0
