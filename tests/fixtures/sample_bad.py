# ruff: noqa
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


def bool_compare(x):
    if x == True:
        return False
    return True


def verbose_bool(x):
    if x > 0:
        return True
    else:
        return False


def redundant_bool(x):
    result = bool(x)
    return result


def always_true():
    assert True


def complex_function(a, b, c, d, e):
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
