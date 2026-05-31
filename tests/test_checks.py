from rabbitinspect import analyze_code, apply_fixes


def check_code(source, expected_codes):
    findings = analyze_code(source)
    codes = {f["code"] for f in findings}
    for code in expected_codes:
        assert code in codes, f"Expected {code} not found in {codes}"
    return findings


def assert_no_findings(source, ignore_codes=None):
    default_ignore = {"RAB001", "RAB090", "RAB091", "RAB092", "RAB093", "RAB094", "RAB095",
                       "RAB096", "RAB097", "RAB098", "RAB099", "RAB100", "RAB103", "RAB104",
                       "RAB105",
                       "RAB106", "RAB107", "RAB108", "RAB109", "RAB110", "RAB111", "RAB112",
                       "RAB113", "RAB114", "RAB115",
                       "RAB118", "RAB119", "RAB120",
                       "RAB123", "RAB124",
                       "RAB126",
                       "RAB127", "RAB128"}
    if ignore_codes is not None:
        ignore_codes = default_ignore | ignore_codes
    else:
        ignore_codes = default_ignore
    findings = analyze_code(source)
    filtered = [f for f in findings if f["code"] not in ignore_codes]
    assert len(filtered) == 0, f"Expected no findings, got: {filtered}"


def finding_by_code(findings, code):
    for f in findings:
        if f["code"] == code:
            return f
    return None


class TestRAB002NoneComparison:
    def test_eq_none(self):
        findings = check_code("if x == None: pass", {"RAB002"})
        f = finding_by_code(findings, "RAB002")
        assert f["fix"]["replacement"] == " is None"

    def test_ne_none(self):
        findings = check_code("if x != None: pass", {"RAB002"})
        f = finding_by_code(findings, "RAB002")
        assert f["fix"]["replacement"] == " is not None"

    def test_is_none_no_warning(self):
        assert_no_findings("if x is None: pass")

    def test_is_not_none_no_warning(self):
        assert_no_findings("if x is not None: pass")


class TestRAB003LenZero:
    def test_len_eq_zero(self):
        findings = check_code("if len(x) == 0: pass", {"RAB003"})
        f = finding_by_code(findings, "RAB003")
        assert "not x" in f["fix"]["replacement"]

    def test_len_ne_zero(self):
        findings = check_code("if len(x) != 0: pass", {"RAB003"})
        f = finding_by_code(findings, "RAB003")
        assert f["fix"]["replacement"] == "x"

    def test_len_gt_zero(self):
        findings = check_code("if len(x) > 0: pass", {"RAB003"})
        f = finding_by_code(findings, "RAB003")
        assert f["fix"]["replacement"] == "x"

    def test_not_len_no_warning(self):
        assert_no_findings("if not x: pass")

    def test_nonzero_constant(self):
        findings = analyze_code("if len(x) == 1: pass")
        assert finding_by_code(findings, "RAB003") is None


class TestRAB004ListToGenerator:
    def test_any_list_comp(self):
        findings = check_code("result = any([x > 0 for x in items])", {"RAB004"})
        f = finding_by_code(findings, "RAB004")
        assert "(" in f["fix"]["replacement"]
        assert ")" in f["fix"]["replacement"]

    def test_all_list_comp(self):
        findings = check_code("result = all([x > 0 for x in items])", {"RAB004"})
        f = finding_by_code(findings, "RAB004")
        assert "(" in f["fix"]["replacement"]

    def test_sum_list_comp(self):
        findings = check_code("result = sum([x for x in items])", {"RAB004"})
        f = finding_by_code(findings, "RAB004")
        assert "(" in f["fix"]["replacement"]

    def test_generator_no_warning(self):
        assert_no_findings("result = any(x > 0 for x in items)")

    def test_list_comp_standalone(self):
        assert_no_findings("result = [x for x in items]")


class TestRAB005DictKeys:
    def test_for_keys(self):
        findings = check_code("for k in d.keys(): pass", {"RAB005"})
        f = finding_by_code(findings, "RAB005")
        assert f is not None

    def test_for_direct(self):
        assert_no_findings("for k in d: pass", ignore_codes={"RAB001", "RAB055"})


class TestRAB006TypeComparison:
    def test_type_eq(self):
        findings = check_code("if type(x) == int: pass", {"RAB006"})
        f = finding_by_code(findings, "RAB006")
        assert "isinstance" in f["fix"]["replacement"]

    def test_type_is(self):
        findings = check_code("if type(x) is int: pass", {"RAB006"})
        assert finding_by_code(findings, "RAB006") is not None

    def test_isinstance_no_warning(self):
        assert_no_findings("if isinstance(x, int): pass")


class TestRAB007UnnecessaryElse:
    def test_unnecessary_else_return(self):
        findings = check_code(
            """
if x > 0:
    return x
else:
    return 0
""",
            {"RAB007"},
        )

    def test_unnecessary_else_raise(self):
        findings = check_code(
            """
if x < 0:
    raise ValueError
else:
    return 0
""",
            {"RAB007"},
        )

    def test_no_false_positive(self):
        assert_no_findings(
            """
if x > 0:
    y = x
else:
    y = 0
return y
"""
        )


class TestApplyFixes:
    def test_fix_none_comparison(self):
        source = "if x == None: pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "is None" in result
        assert "==" not in result.replace("x is None", "")

    def test_fix_len_zero(self):
        source = "if len(items) == 0: pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "not items" in result

    def test_fix_multiple(self):
        source = """
if x == None:
    if len(items) == 0:
        pass
"""
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "is None" in result
        assert "not items" in result

    def test_fix_list_to_generator(self):
        source = "result = any([x > 0 for x in items])"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "any(" in result
        assert "[x > 0" not in result
        assert "(x > 0" in result

    def test_fix_type_comparison(self):
        source = "if type(x) == int: pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "isinstance" in result


class TestEndToEnd:
    def test_sample_bad(self):
        with open("tests/fixtures/sample_bad.py") as f:
            source = f.read()
        findings = analyze_code(source)
        codes = {f["code"] for f in findings}
        assert "RAB002" in codes
        assert "RAB003" in codes
        assert "RAB004" in codes
        assert "RAB005" in codes
        assert "RAB006" in codes
        assert "RAB007" in codes
        assert "RAB029" in codes
        assert "RAB030" in codes
        assert "RAB032" in codes
        assert "RAB034" in codes
        assert "RAB101" in codes

    def test_sample_good(self):
        with open("tests/fixtures/sample_good.py") as f:
            source = f.read()
        findings = analyze_code(source)
        codes = {f["code"] for f in findings}
        assert "RAB002" not in codes
        assert "RAB003" not in codes
        assert "RAB004" not in codes
        assert "RAB005" not in codes
        assert "RAB006" not in codes
        assert "RAB007" not in codes
        assert "RAB029" not in codes
        assert "RAB030" not in codes
        assert "RAB032" not in codes
        assert "RAB034" not in codes


class TestRAB008Enumerate:
    def test_range_len(self):
        check_code("for i in range(len(items)): print(items[i])", {"RAB008"})

    def test_no_warning_direct(self):
        assert_no_findings("for i, item in enumerate(items): print(item)")


class TestRAB009StrConcat:
    def test_str_concat_loop(self):
        check_code(
            """
s = ""
for x in items:
    s += str(x)
""",
            {"RAB009"},
        )

    def test_no_warning(self):
        assert_no_findings('result = "".join(str(x) for x in items)')


class TestRAB010ListSet:
    def test_set_list(self):
        check_code("result = set(list(items))", {"RAB010"})

    def test_no_warning(self):
        assert_no_findings("result = set(items)")


class TestRAB015DictKeysIn:
    def test_keys_in(self):
        check_code('if k in d.keys(): pass', {"RAB015"})

    def test_no_warning(self):
        assert_no_findings('if k in d: pass')


class TestRAB023RedundantCall:
    def test_redundant_call(self):
        check_code("result = obj.call()", {"RAB023"})

    def test_no_warning(self):
        assert_no_findings("result = obj.method()")


class TestRAB029BoolComparison:
    def test_eq_true(self):
        findings = check_code("if x == True: pass", {"RAB029"})
        f = finding_by_code(findings, "RAB029")
        assert "x" in f["fix"]["replacement"]
        assert "not" not in f["fix"]["replacement"]

    def test_eq_false(self):
        findings = check_code("if x == False: pass", {"RAB029"})
        f = finding_by_code(findings, "RAB029")
        assert "not x" in f["fix"]["replacement"]

    def test_ne_true(self):
        findings = check_code("if x != True: pass", {"RAB029"})
        f = finding_by_code(findings, "RAB029")
        assert "not x" in f["fix"]["replacement"]

    def test_ne_false(self):
        findings = check_code("if x != False: pass", {"RAB029"})
        f = finding_by_code(findings, "RAB029")
        assert "x" in f["fix"]["replacement"]
        assert "not" not in f["fix"]["replacement"]

    def test_is_true_no_warning(self):
        assert_no_findings("if x is True: pass", ignore_codes={"RAB001", "RAB053"})

    def test_no_constant_side(self):
        assert_no_findings("if x == 1: pass")


class TestRAB030BoolReturn:
    def test_if_return_true_else_false(self):
        findings = check_code("if x > 0:\n    return True\nelse:\n    return False\n", {"RAB030"})
        f = finding_by_code(findings, "RAB030")
        assert f["fix"]["replacement"] == "return x > 0"

    def test_if_return_false_else_true(self):
        findings = check_code("if x > 0:\n    return False\nelse:\n    return True\n", {"RAB030"})
        f = finding_by_code(findings, "RAB030")
        assert "not" in f["fix"]["replacement"]

    def test_no_else_no_warning(self):
        assert_no_findings("if x > 0:\n    return True\n")

    def test_non_bool_return_no_warning(self):
        assert_no_findings("if x > 0:\n    y = x\nelse:\n    y = 0\n")


class TestRAB032BoolCall:
    def test_bool_call(self):
        findings = check_code("result = bool(x)", {"RAB032"})
        f = finding_by_code(findings, "RAB032")
        assert f["fix"]["replacement"] == "x"

    def test_bool_call_in_if(self):
        findings = check_code("if bool(x): pass", {"RAB032"})
        f = finding_by_code(findings, "RAB032")
        assert f["fix"]["replacement"] == "x"

    def test_no_warning_non_bool(self):
        assert_no_findings("result = int(x)")


class TestRAB034AssertConstant:
    def test_assert_true(self):
        check_code("assert True", {"RAB034"})

    def test_assert_false(self):
        check_code("assert False", {"RAB034"})

    def test_no_warning_assert_expr(self):
        assert_no_findings("assert x > 0")


class TestRAB101CyclomaticComplexity:
    def test_simple_function_no_warning(self):
        assert_no_findings(
            """
def simple(x) -> int:
    return x + 1
"""
        )

    def test_complex_function(self):
        source = """
def complex_func(x, y, z):
    if x > 0:
        for i in range(y):
            if i % 2 == 0:
                print(i)
            elif i % 3 == 0:
                print("three")
            else:
                print("other")
    elif z > 0:
        while z > 0:
            z -= 1
    else:
        try:
            pass
        except ValueError:
            pass
        except TypeError:
            pass
    return x
"""
        check_code(source, {"RAB101"})

    def test_no_warning_simple_conditional(self):
        assert_no_findings(
            """
def check(x) -> bool:
    if x > 0:
        return True
    return False
"""
        )


class TestApplyFixesExisting:
    def test_fix_none_comparison(self):
        source = "if x == None: pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "is None" in result
        assert "==" not in result.replace("x is None", "")

    def test_fix_len_zero(self):
        source = "if len(items) == 0: pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "not items" in result

    def test_fix_multiple(self):
        source = """
if x == None:
    if len(items) == 0:
        pass
"""
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "is None" in result
        assert "not items" in result

    def test_fix_list_to_generator(self):
        source = "result = any([x > 0 for x in items])"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "any(" in result
        assert "[x > 0" not in result
        assert "(x > 0" in result

    def test_fix_type_comparison(self):
        source = "if type(x) == int: pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "isinstance" in result

    def test_fix_bool_comparison(self):
        source = "if x == True: pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "x" in result
        assert "== True" not in result

    def test_fix_bool_return(self):
        source = "if x > 0:\n    return True\nelse:\n    return False\n"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f["code"] == "RAB030"]
        result = apply_fixes(source, fixes)
        assert "return x > 0" in result
        assert "if" not in result

    def test_fix_bool_call(self):
        source = "result = bool(x)"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "= x" in result

    def test_fix_dict_keys_loop(self):
        source = "for item in d.keys():\n    print(item)"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "for item in d:" in result
        assert ".keys()" not in result

    def test_fix_dict_keys_in(self):
        source = "if k in d.keys(): pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "k in d" in result
        assert ".keys()" not in result

    def test_fix_assert_true(self):
        source = "assert True\nx = 1"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert result.strip() == "x = 1"

    def test_fix_assert_false(self):
        source = "assert False"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "raise AssertionError" in result

    def test_fix_unnecessary_else(self):
        source = "if x > 0:\n    return x\nelse:\n    return 0\n"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "else:" not in result
        assert "return 0" in result


class TestRAB011MutableDefault:
    def test_list_default(self):
        check_code("def foo(x=[]): pass", {"RAB011"})

    def test_dict_default(self):
        check_code("def foo(x={}): pass", {"RAB011"})

    def test_set_default(self):
        check_code("def foo(x={1}): pass", {"RAB011"})

    def test_no_warning_none_default(self):
        assert_no_findings("def foo(x=None) -> int: pass")

    def test_no_warning_int_default(self):
        assert_no_findings("def foo(x=5) -> int: pass")

    def test_async_func(self):
        check_code("async def foo(x=[]): pass", {"RAB011"})


class TestRAB012BareExcept:
    def test_bare_except(self):
        check_code("try:\n    pass\nexcept:\n    pass", {"RAB012"})

    def test_no_warning_typed_except(self):
        assert_no_findings("try:\n    pass\nexcept ValueError:\n    pass", ignore_codes={"RAB001", "RAB078"})

    def test_no_warning_except_as(self):
        assert_no_findings("try:\n    pass\nexcept Exception as e:\n    pass", ignore_codes={"RAB001", "RAB078"})


class TestRAB013BareExceptPass:
    def test_bare_except_pass(self):
        check_code("try:\n    pass\nexcept:\n    pass", {"RAB013"})

    def test_no_warning_except_with_action(self):
        assert_no_findings("try:\n    pass\nexcept ValueError:\n    pass", ignore_codes={"RAB001", "RAB078"})


class TestRAB014ClassObject:
    def test_class_object(self):
        check_code("class Foo(object): pass", {"RAB014"})

    def test_no_warning_no_base(self):
        assert_no_findings("class Foo: pass")

    def test_no_warning_other_base(self):
        assert_no_findings("class Foo(Base): pass")

    def test_no_warning_multiple_bases(self):
        assert_no_findings("class Foo(Base, metaclass=ABCMeta): pass")


class TestRAB016FormatCall:
    def test_format_call(self):
        check_code('result = "Hello {}".format(name)', {"RAB016"})

    def test_no_warning_fstring(self):
        assert_no_findings('result = f"Hello {name}"')

    def test_no_warning_no_args_format(self):
        assert_no_findings('result = "Hello".format()')

    def test_multiple_args(self):
        check_code('result = "{a} {b}".format(a, b)', {"RAB016"})


class TestRAB017FunctionLength:
    def test_short_function_no_warning(self):
        assert_no_findings(
            """
def short(x) -> int:
    y = x + 1
    return y
"""
        )

    def test_long_function(self):
        lines = "\n".join(f"    x{i} = {i}" for i in range(35))
        source = f"def long_func():\n{lines}\n"
        check_code(source, {"RAB017"})

    def test_long_method_no_warning_outside_func(self):
        source = "\n".join(f"x{i} = {i}" for i in range(35))
        assert_no_findings(source)


class TestRAB018TooManyParams:
    def test_too_many_params(self):
        check_code("def foo(a, b, c, d, e, f, g): pass", {"RAB018"})

    def test_no_warning_six_params(self):
        assert_no_findings("def foo(a, b, c, d, e, f) -> None: pass")

    def test_no_warning_one_param(self):
        assert_no_findings("def foo(x) -> None: pass")


class TestRAB019OsSystem:
    def test_os_system(self):
        check_code("import os; os.system('ls')", {"RAB019"})

    def test_no_warning_other_call(self):
        assert_no_findings("import os; os.listdir('.')")


class TestRAB020TimeTime:
    def test_time_time(self):
        check_code("import time; t = time.time()", {"RAB020"})

    def test_no_warning_other_call(self):
        assert_no_findings("import time; t = time.sleep(1)")


class TestRAB022MissingReturnHint:
    def test_missing_return_hint(self):
        check_code("def foo(x): return x", {"RAB022"})

    def test_no_warning_with_hint(self):
        assert_no_findings("def foo(x: int) -> int: return x")

    def test_no_warning_private_func(self):
        assert_no_findings("def _internal(x): return x")

    def test_async_func(self):
        check_code("async def fetch(url): return None", {"RAB022"})


class TestRAB024DeepComprehension:
    def test_deep_list_comp(self):
        check_code("result = [[x for y in z for w in z for x in w] for z in items]", {"RAB024"})

    def test_no_warning_simple_comp(self):
        assert_no_findings("result = [x for x in items]")

    def test_no_warning_two_levels(self):
        assert_no_findings("result = [x for y in z for x in y]")

    def test_deep_generator(self):
        check_code("result = (x for y in z for w in z for x in w)", {"RAB024"})


class TestRAB025LongIfChain:
    def test_long_if_chain(self):
        source = """
if x == 1:
    pass
elif x == 2:
    pass
elif x == 3:
    pass
elif x == 4:
    pass
"""
        check_code(source, {"RAB025"})

    def test_no_warning_short_chain(self):
        source = """
if x == 1:
    pass
elif x == 2:
    pass
elif x == 3:
    pass
"""
        assert_no_findings(source)

    def test_no_warning_single_if(self):
        assert_no_findings("if x > 0: pass")


class TestRAB102CognitiveComplexity:
    def test_simple_function_no_warning(self):
        assert_no_findings(
            """
def simple(x) -> int:
    return x + 1
"""
        )
    def test_high_cognitive_complexity(self):
        source = """
def complex_func(x, y, z):
    if x > 0:
        for i in range(y):
            if i % 2 == 0:
                print(i)
            elif i % 3 == 0:
                print("three")
            else:
                print("other")
    elif z > 0:
        while z > 0:
            if z == 5:
                break
            z -= 1
    else:
        try:
            pass
        except ValueError:
            pass
        except TypeError:
            pass
    return x
"""
        check_code(source, {"RAB102"})


class TestRAB097DebugLeftover:
    def test_print_call(self):
        check_code("print('hello')", {"RAB097"})

    def test_print_with_args(self):
        check_code("print(x, y, z)", {"RAB097"})

    def test_breakpoint_call(self):
        check_code("breakpoint()", {"RAB097"})

    def test_pdb_set_trace(self):
        check_code("pdb.set_trace()", {"RAB097"})

    def test_no_warning_normal_call(self):
        assert_no_findings("logger.info('hello')", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_print_in_string(self):
        assert_no_findings('x = "print is a function"', ignore_codes={"RAB001", "RAB022"})


class TestRAB098ImportInFunction:
    def test_import_in_function(self):
        check_code("def foo():\n    import os", {"RAB098"})

    def test_from_import_in_function(self):
        check_code("def foo():\n    from os import path", {"RAB098"})

    def test_no_warning_module_level(self):
        assert_no_findings("import os", ignore_codes={"RAB001"})

    def test_no_warning_nested_class(self):
        check_code("class Foo:\n    import os", {"RAB098"})


class TestRAB099DuplicateKey:
    def test_duplicate_dict_key(self):
        check_code("x = {'a': 1, 'b': 2, 'a': 3}", {"RAB099"})

    def test_duplicate_set_element(self):
        check_code("x = {1, 2, 3, 1}", {"RAB099"})

    def test_duplicate_str_set(self):
        check_code("x = {'a', 'b', 'a'}", {"RAB099"})

    def test_no_warning_unique_dict(self):
        assert_no_findings("x = {'a': 1, 'b': 2, 'c': 3}", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_unique_set(self):
        assert_no_findings("x = {1, 2, 3}", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_variable_keys(self):
        assert_no_findings("x = {a: 1, b: 2}", ignore_codes={"RAB001", "RAB022"})


class TestRAB106BroadExcept:
    def test_broad_except_exception(self):
        check_code("try:\n    pass\nexcept Exception:\n    pass", {"RAB106"})

    def test_broad_except_as(self):
        check_code("try:\n    pass\nexcept Exception as e:\n    pass", {"RAB106"})

    def test_no_warning_specific_except(self):
        assert_no_findings("try:\n    pass\nexcept ValueError:\n    pass", ignore_codes={"RAB001", "RAB078"})

    def test_no_warning_multiple_except(self):
        assert_no_findings("try:\n    pass\nexcept (ValueError, TypeError):\n    pass", ignore_codes={"RAB001", "RAB078"})


class TestRAB090MissingParamType:
    def test_missing_param_type(self):
        check_code("def foo(x): pass", {"RAB090"})

    def test_missing_param_type_multiple(self):
        check_code("def foo(x, y): pass", {"RAB090"})

    def test_no_warning_with_annotation(self):
        assert_no_findings("def foo(x: int) -> None: pass", ignore_codes={"RAB001"})

    def test_no_warning_private_func(self):
        assert_no_findings("def _internal(x) -> None: pass")

    def test_async_func(self):
        check_code("async def fetch(url): pass", {"RAB090"})

    def test_missing_some_params(self):
        findings = check_code("def foo(a: int, b): pass", {"RAB090"})
        codes = {f["code"] for f in findings}
        assert "RAB090" in codes


class TestRAB091MissingReturnType:
    def test_missing_return_type(self):
        check_code("def foo(): pass", {"RAB091"})

    def test_missing_return_type_private(self):
        check_code("def _internal(): pass", {"RAB091"})

    def test_no_warning_with_return(self):
        assert_no_findings("def foo() -> None: pass", ignore_codes={"RAB001"})

    def test_async_func(self):
        check_code("async def fetch(): return None", {"RAB091"})


class TestRAB092MissingAttrType:
    def test_missing_attr_type(self):
        check_code("class Foo:\n    x = 1", {"RAB092"})

    def test_no_warning_annotated(self):
        assert_no_findings("class Foo:\n    x: int = 1", ignore_codes={"RAB001"})

    def test_no_warning_private_attr(self):
        assert_no_findings("class Foo:\n    _x = 1")

    def test_no_warning_outside_class(self):
        assert_no_findings("x = 1", ignore_codes={"RAB093"})


class TestRAB093MissingModuleVarType:
    def test_missing_module_var_type(self):
        check_code("x = 1", {"RAB093"})

    def test_no_warning_annotated(self):
        assert_no_findings("x: int = 1", ignore_codes={"RAB001"})

    def test_no_warning_private_var(self):
        assert_no_findings("_x = 1")


class TestRAB094AnyAnnotation:
    def test_any_type(self):
        check_code("x: Any = 1", {"RAB094"})

    def test_any_in_generic(self):
        check_code("x: list[Any] = []", {"RAB094"})

    def test_no_warning_concrete_type(self):
        assert_no_findings("x: int = 1", ignore_codes={"RAB001"})

    def test_no_warning_no_any(self):
        assert_no_findings("def foo(x: str) -> None: pass", ignore_codes={"RAB001"})


class TestRAB096UnusedImport:
    def test_unused_import(self):
        check_code("import os", {"RAB096"})

    def test_unused_from_import(self):
        check_code("from os import path", {"RAB096"})

    def test_no_warning_used_import(self):
        assert_no_findings("import os\nx = os.getcwd()", ignore_codes={"RAB001"})

    def test_no_warning_used_from_import(self):
        assert_no_findings("from os import getcwd\nx = getcwd()", ignore_codes={"RAB001"})

    def test_no_warning_aliased_import(self):
        assert_no_findings("import numpy as np\nx = np.array([1])", ignore_codes={"RAB001"})


class TestRAB105InconsistentReturn:
    def test_inconsistent_return(self):
        check_code("""
def foo(x):
    if x > 0:
        return x
    return
""", {"RAB105"})

    def test_inconsistent_return_async(self):
        check_code("""
async def fetch(x):
    if x > 0:
        return x
    return
""", {"RAB105"})

    def test_no_warning_consistent_value(self):
        assert_no_findings("""
def foo(x) -> int:
    if x > 0:
        return x
    return 0
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_bare_only(self):
        assert_no_findings("""
def foo(x) -> None:
    if x > 0:
        return
    return
""", ignore_codes={"RAB001"})

    def test_no_warning_no_return(self):
        assert_no_findings("""
def foo(x) -> None:
    pass
""", ignore_codes={"RAB001"})


class TestRAB108AllExport:
    def test_non_string_in_all(self):
        check_code("__all__ = ['foo', 42]", {"RAB108"})

    def test_variable_in_all(self):
        check_code("x = 'bar'\n__all__ = ['foo', x]", {"RAB108"})

    def test_no_warning_all_strings(self):
        assert_no_findings("__all__ = ['foo', 'bar']", ignore_codes={"RAB001"})

    def test_no_warning_tuple_all(self):
        assert_no_findings("__all__ = ('foo', 'bar')", ignore_codes={"RAB001"})


class TestRAB100RedundantElif:
    def test_redundant_elif(self):
        check_code("""
if x > 0:
    return x
elif x == 0:
    pass
elif x < 0:
    pass
""", {"RAB100"})

    def test_no_warning_no_elif(self):
        assert_no_findings("""
if x > 0:
    return x
else:
    return 0
""", ignore_codes={"RAB007"})

    def test_no_warning_not_terminal(self):
        assert_no_findings("""
if x > 0:
    print(1)
elif x == 0:
    pass
""")


class TestRAB103SelfComparison:
    def test_self_eq(self):
        check_code("if x == x: pass", {"RAB103"})

    def test_self_not_eq(self):
        check_code("if x != x: pass", {"RAB103"})

    def test_self_gt(self):
        check_code("if x > x: pass", {"RAB103"})

    def test_no_warning_normal(self):
        assert_no_findings("if x == y: pass")

    def test_no_warning_constant(self):
        assert_no_findings("if 1 == 1: pass")


class TestRAB104PassThroughGen:
    def test_list_passthrough(self):
        check_code("result = list(x for x in items)", {"RAB104"})

    def test_set_passthrough(self):
        check_code("result = set(x for x in items)", {"RAB104"})

    def test_tuple_passthrough(self):
        check_code("result = tuple(x for x in items)", {"RAB104"})

    def test_frozenset_passthrough(self):
        check_code("result = frozenset(x for x in items)", {"RAB104"})

    def test_no_warning_transform(self):
        assert_no_findings("result = list(x * 2 for x in items)")

    def test_no_warning_direct(self):
        assert_no_findings("result = list(items)")

    def test_no_warning_filter(self):
        assert_no_findings("result = list(x for x in items if x > 0)")


class TestRAB107TodoComment:
    def test_todo_comment(self):
        check_code("# TODO: fix this later\nx = 1", {"RAB107"})

    def test_fixme_comment(self):
        check_code("# FIXME: this is broken\nx = 1", {"RAB107"})

    def test_hack_comment(self):
        check_code("# HACK: workaround\nx = 1", {"RAB107"})

    def test_xxx_comment(self):
        check_code("# XXX: review\nx = 1", {"RAB107"})

    def test_no_warning_normal_comment(self):
        assert_no_findings("# normal comment\nx = 1")

    def test_no_warning_no_comment(self):
        assert_no_findings("x = 1", ignore_codes={"RAB093"})


class TestRAB109ClassName:
    def test_snake_case_class(self):
        check_code("class my_class: pass", {"RAB109"})

    def test_lowercase_class(self):
        check_code("class myclass: pass", {"RAB109"})

    def test_no_warning_camelcase(self):
        assert_no_findings("class MyClass: pass")

    def test_no_warning_single_upper(self):
        assert_no_findings("class A: pass")


class TestRAB110FunctionName:
    def test_camelcase_function(self):
        check_code("def MyFunc(): pass", {"RAB110"})

    def test_pascalcase_function(self):
        check_code("def MyFunction(): pass", {"RAB110"})

    def test_no_warning_snake_case(self):
        assert_no_findings("def my_function() -> None: pass", ignore_codes={"RAB001"})

    def test_no_warning_dunder(self):
        assert_no_findings("def __init__(self) -> None: pass", ignore_codes={"RAB001"})

    def test_no_warning_private(self):
        assert_no_findings("def _helper() -> None: pass", ignore_codes={"RAB001"})

    def test_async_func(self):
        check_code("async def MyFunc(): pass", {"RAB110"})

    def test_no_warning_already_underscore(self):
        assert_no_findings("def my_func() -> None: pass", ignore_codes={"RAB001"})


class TestRAB111ConstantName:
    def test_lowercase_constant(self):
        check_code("my_const = 42", {"RAB111"})

    def test_no_warning_uppercase(self):
        assert_no_findings("MY_CONST = 42", ignore_codes={"RAB001"})

    def test_no_warning_non_constant(self):
        assert_no_findings("my_var = get_value()", ignore_codes={"RAB001", "RAB093"})

    def test_no_warning_private(self):
        assert_no_findings("_private = 42")

    def test_no_warning_inside_function(self):
        assert_no_findings("def foo():\n    my_var = 42", ignore_codes={"RAB001", "RAB022"})


class TestRAB095TypeDefaultMismatch:
    def test_none_default_with_concrete_type(self):
        check_code("def foo(x: str = None): pass", {"RAB095"})

    def test_mutable_default_with_type(self):
        check_code("def foo(x: list = []): pass", {"RAB095"})

    def test_no_warning_optional_type(self):
        assert_no_findings("def foo(x: Optional[int] = None) -> None: pass", ignore_codes={"RAB001", "RAB022", "RAB044", "RAB090"})

    def test_no_warning_correct_default(self):
        assert_no_findings("def foo(x: int = 0) -> None: pass", ignore_codes={"RAB001"})

    def test_no_warning_any_default(self):
        assert_no_findings("def foo(x: Any = None) -> None: pass", ignore_codes={"RAB001", "RAB022", "RAB090"})


class TestRAB112UnnecessaryPass:
    def test_pass_after_code(self):
        findings = check_code("def foo():\n    x = 1\n    pass", {"RAB112"})
        f = finding_by_code(findings, "RAB112")
        assert f["fix"] is not None

    def test_pass_in_while(self):
        check_code("while True:\n    print(1)\n    pass\n    break", {"RAB112"})

    def test_pass_in_if(self):
        check_code("if x > 0:\n    print(1)\n    pass", {"RAB112"})

    def test_no_warning_pass_only(self):
        assert_no_findings("def foo():\n    pass", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_empty_func(self):
        # When there's a docstring AND pass, the pass is NOT needed (RAB062 handles docstring)
        # RAB112 also flags pass when body has other stmts - docstring counts as a stmt
        check_code("def foo():\n    \"\"\"doc\"\"\"\n    pass", {"RAB112"})


class TestRAB113EvalExec:
    def test_eval(self):
        check_code("eval('print(1)')", {"RAB113"})

    def test_exec(self):
        check_code("exec('print(1)')", {"RAB113"})

    def test_no_warning_normal_call(self):
        assert_no_findings("print(1)")


class TestRAB114PickleLoad:
    def test_pickle_load(self):
        check_code("pickle.load(f)", {"RAB114"})

    def test_pickle_loads(self):
        check_code("pickle.loads(data)", {"RAB114"})

    def test_no_warning_other_call(self):
        assert_no_findings("json.loads(data)")


class TestRAB115YamlLoad:
    def test_yaml_load(self):
        check_code("yaml.load(data)", {"RAB115"})

    def test_no_warning_safe_load(self):
        assert_no_findings("yaml.safe_load(data)")

    def test_no_warning_with_loader(self):
        assert_no_findings("yaml.load(data, Loader=yaml.SafeLoader)")


class TestRAB118DelExceptVar:
    def test_del_except_var(self):
        check_code("""
try:
    pass
except Exception as e:
    del e
""", {"RAB118"})

    def test_no_warning_no_del(self):
        assert_no_findings("""
try:
    pass
except Exception as e:
    print(e)
""", ignore_codes={"RAB001", "RAB106"})


class TestRAB119SuperInit:
    def test_missing_super_init(self):
        check_code("""
class Base:
    def __init__(self):
        pass
class Child(Base):
    def __init__(self):
        self.x = 1
""", {"RAB119"})

    def test_no_warning_with_super(self):
        assert_no_findings("""
class Base:
    def __init__(self):
        pass
class Child(Base):
    def __init__(self):
        super().__init__()
        self.x = 1
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_no_base(self):
        assert_no_findings("""
class Foo:
    def __init__(self):
        self.x = 1
""", ignore_codes={"RAB001"})


class TestRAB120ModifyIter:
    def test_list_remove_in_loop(self):
        check_code("for x in items:\n    items.remove(x)", {"RAB120"})

    def test_dict_del_in_loop(self):
        check_code("for k in d:\n    del d[k]", {"RAB120"})

    def test_list_pop_in_loop(self):
        check_code("for x in items:\n    items.pop(0)", {"RAB120"})

    def test_no_warning_different_iter(self):
        assert_no_findings("""
for x in items:
    other.remove(x)
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_read_only(self):
        assert_no_findings("""
for x in items:
    print(x)
""", ignore_codes={"RAB001", "RAB022"})


class TestRAB123DeprecatedAsyncio:
    def test_get_event_loop(self):
        check_code("asyncio.get_event_loop()", {"RAB123"})

    def test_ensure_future(self):
        check_code("asyncio.ensure_future(coro)", {"RAB123"})

    def test_no_warning_create_task(self):
        assert_no_findings("asyncio.create_task(coro)")


class TestRAB124AsyncBlocking:
    def test_sleep_in_async(self):
        check_code("""
async def foo():
    time.sleep(1)
""", {"RAB124"})

    def test_subprocess_in_async(self):
        check_code("""
async def foo():
    subprocess.run(['ls'])
""", {"RAB124"})

    def test_input_in_async(self):
        check_code("""
async def foo():
    input()
""", {"RAB124"})

    def test_no_warning_sync_func(self):
        assert_no_findings("""
def foo():
    time.sleep(1)
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_async_ok(self):
        assert_no_findings("""
async def foo():
    await asyncio.sleep(1)
""", ignore_codes={"RAB001", "RAB022"})


class TestRAB126MagicNumber:
    def test_magic_number(self):
        check_code("""
def foo():
    x = 42
""", {"RAB126"})

    def test_no_warning_allowed_zero(self):
        assert_no_findings("""
def foo():
    x = 0
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_allowed_one(self):
        assert_no_findings("""
def foo():
    x = 1
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_module_level(self):
        assert_no_findings("x = 42", ignore_codes={"RAB001", "RAB093"})


class TestRAB127LoopElse:
    def test_break_with_else(self):
        check_code("""
for x in items:
    if x > 0:
        break
else:
    print('not found')
""", {"RAB127"})

    def test_while_break_with_else(self):
        check_code("""
while True:
    break
else:
    print('done')
""", {"RAB127"})

    def test_no_warning_no_break(self):
        assert_no_findings("""
for x in items:
    print(x)
else:
    print('done')
""", ignore_codes={"RAB001", "RAB022", "RAB055"})

    def test_no_warning_no_else(self):
        assert_no_findings("""
for x in items:
    if x > 0:
        break
""", ignore_codes={"RAB001", "RAB022", "RAB055"})


class TestRAB128NotIn:
    def test_not_in(self):
        findings = check_code("if not x in y: pass", {"RAB128"})
        f = finding_by_code(findings, "RAB128")
        assert f["fix"]["replacement"] == "x not in y"

    def test_no_warning_not_in(self):
        assert_no_findings("if x not in y: pass")


class TestEndToEndAll:
    def test_all_checks_bad(self):
        with open("tests/fixtures/all_checks_bad.py") as f:
            source = f.read()
        findings = analyze_code(source)
        codes = {f["code"] for f in findings}
        all_codes = {"RAB002", "RAB003", "RAB004", "RAB005", "RAB006", "RAB007",
                     "RAB008", "RAB009", "RAB010", "RAB011", "RAB012", "RAB013",
                     "RAB014", "RAB015", "RAB016", "RAB017", "RAB018", "RAB019",
                     "RAB020", "RAB022", "RAB023", "RAB024", "RAB025",
                     "RAB026", "RAB029", "RAB030", "RAB031", "RAB032",
                     "RAB034", "RAB035", "RAB036", "RAB037", "RAB038",
                     "RAB039", "RAB040", "RAB041", "RAB042",
                     "RAB043", "RAB044", "RAB045",
                     "RAB090", "RAB091", "RAB092", "RAB093",
                     "RAB096",
                     "RAB097", "RAB098", "RAB099",
                     "RAB100",
                     "RAB101", "RAB102",
                     "RAB103", "RAB104",
                     "RAB105",
                     "RAB106",
                     "RAB107",
                     "RAB108",
                     "RAB109",
                     "RAB110", "RAB111",
                     "RAB112",
                     "RAB113", "RAB114", "RAB115",
                     "RAB118",
                     "RAB120",
                     "RAB123", "RAB124",
                     "RAB126",
                     "RAB127", "RAB128"}
        for code in all_codes:
            assert code in codes, f"Expected {code} not found in {codes}"

    def test_all_checks_good(self):
        with open("tests/fixtures/all_checks_good.py") as f:
            source = f.read()
        findings = analyze_code(source)
        codes = {f["code"] for f in findings}
        forbidden_in_good = {"RAB002", "RAB003", "RAB004", "RAB005", "RAB006", "RAB007",
                             "RAB008", "RAB009", "RAB010", "RAB011", "RAB012", "RAB013",
                             "RAB014", "RAB015", "RAB016", "RAB017", "RAB018", "RAB019",
                             "RAB020", "RAB022", "RAB023", "RAB024", "RAB025",
                             "RAB026", "RAB029", "RAB030", "RAB031", "RAB032",
                             "RAB034", "RAB035", "RAB036", "RAB037", "RAB038",
                             "RAB039", "RAB040", "RAB041", "RAB042",
                             "RAB043", "RAB044", "RAB045",
                             "RAB098", "RAB099",
                             "RAB101", "RAB102"}
        for code in forbidden_in_good:
            if code in {"RAB001", "RAB090", "RAB091", "RAB092", "RAB093", "RAB094", "RAB095",
                         "RAB096", "RAB097", "RAB098", "RAB100", "RAB101", "RAB102", "RAB103", "RAB104",
                         "RAB105",
                         "RAB106", "RAB107", "RAB108", "RAB109", "RAB110", "RAB111", "RAB112",
                         "RAB113", "RAB114", "RAB115",
                         "RAB118", "RAB119", "RAB120",
                         "RAB123", "RAB124",
                         "RAB126",
                         "RAB127", "RAB128"}:
                continue
            assert code not in codes, f"Unexpected {code} found in {codes}"


class TestRAB036PowerOpt:
    def test_square_binop(self):
        findings = check_code("def f(x): return x**2", {"RAB036"})
        f = finding_by_code(findings, "RAB036")
        assert f["fix"]["replacement"] == "x * x"

    def test_cube_binop(self):
        findings = check_code("def f(x): return x**3", {"RAB036"})
        f = finding_by_code(findings, "RAB036")
        assert f["fix"]["replacement"] == "x * x * x"

    def test_mathpow_square(self):
        findings = check_code("import math; math.pow(x, 2)", {"RAB036"})
        f = finding_by_code(findings, "RAB036")
        assert "x * x" in f["fix"]["replacement"]

    def test_mathpow_cube(self):
        findings = check_code("import math; math.pow(y, 3)", {"RAB036"})
        f = finding_by_code(findings, "RAB036")
        assert "y * y * y" in f["fix"]["replacement"]

    def test_no_warning_higher_exp(self):
        assert_no_findings("def f(x) -> int: return x**10")

    def test_no_warning_addition(self):
        assert_no_findings("def f(x) -> int: return x + x")


class TestRAB037MapLambda:
    def test_map_lambda(self):
        findings = check_code("list(map(lambda x: x+1, items))", {"RAB037"})
        f = finding_by_code(findings, "RAB037")
        assert "x+1 for x in items" in f["fix"]["replacement"]

    def test_filter_lambda(self):
        findings = check_code("list(filter(lambda x: x>0, items))", {"RAB037"})
        f = finding_by_code(findings, "RAB037")
        assert "x for x in items if x>0" in f["fix"]["replacement"]

    def test_no_warning_list_comp(self):
        assert_no_findings("result = [x+1 for x in items]")

    def test_no_warning_map_named(self):
        assert_no_findings("result = map(str, items)")


class TestRAB038ListToSet:
    def test_list_in(self):
        findings = check_code("x in [1, 2, 3]", {"RAB038"})
        f = finding_by_code(findings, "RAB038")
        assert f["fix"]["replacement"] == "{1, 2, 3}"

    def test_tuple_not_in(self):
        findings = check_code("x not in (1, 2)", {"RAB038"})
        f = finding_by_code(findings, "RAB038")
        assert f["fix"]["replacement"] == "{1, 2}"

    def test_no_warning_non_const_list(self):
        assert_no_findings("x in [a, b, c]")

    def test_no_warning_dict_access(self):
        assert_no_findings("x in y")


class TestRAB040DataclassSlots:
    def test_dataclass_no_slots(self):
        check_code("""
from dataclasses import dataclass

@dataclass
class Point:
    x: int = 0
""", {"RAB040"})

    def test_dataclass_with_slots(self):
        assert_no_findings("""
from dataclasses import dataclass

@dataclass(slots=True)
class Point:
    x: int = 0
""")

    def test_no_warning_regular_class(self):
        assert_no_findings("class Foo: pass")

    def test_fix_adds_slots(self):
        findings = analyze_code("""
from dataclasses import dataclass

@dataclass
class Point:
    x: int = 0
""")
        f = finding_by_code(findings, "RAB040")
        assert f["fix"]["replacement"] == "dataclass(slots=True)"


class TestRAB041ReCompile:
    def test_recompile_in_function(self):
        check_code("""
import re

def f():
    return re.compile('[a-z]')
""", {"RAB041"})

    def test_no_warning_module_level(self):
        assert_no_findings("""
import re
pattern = re.compile('[a-z]')
""")

    def test_no_warning_no_compile(self):
        assert_no_findings("""
import re

def f() -> None:
    return re.match('[a-z]', 'a')
""")


class TestRAB042Readlines:
    def test_for_readlines(self):
        findings = check_code("for line in f.readlines(): pass", {"RAB042"})
        f = finding_by_code(findings, "RAB042")
        assert f["fix"]["replacement"] == ""

    def test_no_warning_for_direct(self):
        assert_no_findings("for line in f: pass", ignore_codes={"RAB001", "RAB055"})

    def test_no_warning_method_call(self):
        assert_no_findings("data = f.read()")


class TestRAB044TypeUnion:
    def test_optional_in_function_return(self):
        check_code("""
from typing import Optional
def f() -> Optional[int]:
    return None
""", {"RAB044"})

    def test_optional_in_annotation(self):
        check_code("""
from typing import Optional
def f(x: Optional[int]) -> None:
    pass
""", {"RAB044"})

    def test_union_in_annotation(self):
        check_code("""
from typing import Union
x: Union[int, str] = 1
""", {"RAB044"})

    def test_no_warning_str_annotation(self):
        assert_no_findings('def f(x: int) -> None: pass')

    def test_fix_optional(self):
        findings = analyze_code("""
from typing import Optional
x: Optional[int] = None
""")
        f = finding_by_code(findings, "RAB044")
        assert f["fix"]["replacement"] == "int | None"

    def test_fix_union(self):
        findings = analyze_code("""
from typing import Union
x: Union[int, str] = 1
""")
        f = finding_by_code(findings, "RAB044")
        assert f["fix"]["replacement"] == "int | str"


class TestRAB026SortedList:
    def test_sorted_list(self):
        check_code("sorted(list(x))", {"RAB026"})

    def test_sorted_tuple(self):
        check_code("sorted(tuple(x))", {"RAB026"})

    def test_reversed_list(self):
        check_code("reversed(list(x))", {"RAB026"})

    def test_no_warning_sorted_direct(self):
        assert_no_findings("sorted(x)")

    def test_fix_removes_list(self):
        findings = analyze_code("sorted(list(x))")
        f = finding_by_code(findings, "RAB026")
        assert f["fix"]["replacement"] == "sorted(x)"

    def test_fix_removes_tuple(self):
        findings = analyze_code("reversed(tuple(items))")
        f = finding_by_code(findings, "RAB026")
        assert f["fix"]["replacement"] == "reversed(items)"


class TestRAB031DictGet:
    def test_dict_get_return(self):
        check_code("""
def f(d, k):
    if k in d:
        return d[k]
""", {"RAB031"})

    def test_dict_get_assign(self):
        check_code("""
def f(d, k):
    if k in d:
        v = d[k]
""", {"RAB031"})

    def test_no_warning_different_dict(self):
        assert_no_findings("""
def f(d, k):
    if k in d:
        return d2[k]
""", ignore_codes={"RAB001", "RAB022"})

    def test_fix_get_replacement(self):
        findings = analyze_code("""
def f(d, k):
    if k in d:
        return d[k]
""")
        f = finding_by_code(findings, "RAB031")
        assert f["fix"]["replacement"] == "d.get(k)"


class TestRAB035SliceCopy:
    def test_slice_copy(self):
        check_code("x[:]", {"RAB035"})

    def test_no_warning_non_empty_slice(self):
        assert_no_findings("x[1:]", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_subscript(self):
        assert_no_findings("x[0]", ignore_codes={"RAB001", "RAB022"})

    def test_fix_copy(self):
        findings = analyze_code("x[:]")
        f = finding_by_code(findings, "RAB035")
        assert f["fix"]["replacement"] == ".copy()"


class TestRAB039NativeGeneric:
    def test_list_generic(self):
        check_code("x: List[int] = []", {"RAB039"})

    def test_dict_generic(self):
        check_code("x: Dict[str, int] = {}", {"RAB039"})

    def test_tuple_generic(self):
        check_code("x: Tuple[int, ...] = ()", {"RAB039"})

    def test_set_generic(self):
        check_code("x: Set[str] = set()", {"RAB039"})

    def test_frozenset_generic(self):
        check_code("x: FrozenSet[int] = frozenset()", {"RAB039"})

    def test_type_generic(self):
        check_code("x: Type[Base] = Base", {"RAB039"})

    def test_no_warning_non_generic(self):
        assert_no_findings("x: int = 1", ignore_codes={"RAB001"})

    def test_fix_list(self):
        findings = analyze_code("x: List[int] = []")
        f = finding_by_code(findings, "RAB039")
        assert f["fix"]["replacement"] == "list"

    def test_fix_dict(self):
        findings = analyze_code("x: Dict[str, int] = {}")
        f = finding_by_code(findings, "RAB039")
        assert f["fix"]["replacement"] == "dict"


class TestRAB043ManualList:
    def test_manual_list_append(self):
        check_code("""
def f(items):
    result = []
    for x in items:
        result.append(x)
""", {"RAB043"})

    def test_no_warning_comprehension(self):
        assert_no_findings("""
def f(items):
    result = [x for x in items]
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_no_append(self):
        assert_no_findings("""
def f(items):
    result = []
    for x in items:
        print(x)
""", ignore_codes={"RAB001", "RAB022"})


class TestRAB045OpenContext:
    def test_open_expr(self):
        check_code("open('file')", {"RAB045"})

    def test_open_assign(self):
        check_code("f = open('file')", {"RAB045"})

    def test_no_warning_with_open(self):
        assert_no_findings("""
def f():
    with open('file') as fh:
        pass
""", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_other_call(self):
        assert_no_findings("read('file')", ignore_codes={"RAB001", "RAB022"})


class TestRAB046SortedIndex0:
    def test_sorted_index0(self):
        findings = check_code("x = sorted(y)[0]", {"RAB046"})
        f = finding_by_code(findings, "RAB046")
        assert f["fix"]["replacement"] == "min(y)"

    def test_sorted_index0_with_key(self):
        findings = check_code("x = sorted(y, key=len)[0]", {"RAB046"})
        f = finding_by_code(findings, "RAB046")
        assert f["fix"]["replacement"] == "min(y, key=len)"

    def test_sorted_reverse_no_warning(self):
        assert_no_findings("x = sorted(y, reverse=True)[0]", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_regular_subscript(self):
        assert_no_findings("x = y[0]", ignore_codes={"RAB001", "RAB022"})


class TestRAB047SortedIndexNeg1:
    def test_sorted_index_neg1(self):
        findings = check_code("x = sorted(y)[-1]", {"RAB047"})
        f = finding_by_code(findings, "RAB047")
        assert f["fix"]["replacement"] == "max(y)"

    def test_sorted_index_neg1_with_key(self):
        findings = check_code("x = sorted(y, key=len)[-1]", {"RAB047"})
        f = finding_by_code(findings, "RAB047")
        assert f["fix"]["replacement"] == "max(y, key=len)"

    def test_sorted_reverse_no_warning(self):
        assert_no_findings("x = sorted(y, reverse=True)[-1]", ignore_codes={"RAB001", "RAB022"})


class TestRAB048NotIsNone:
    def test_not_is_none(self):
        findings = check_code("if not x is None: pass", {"RAB048"})
        f = finding_by_code(findings, "RAB048")
        assert f["fix"]["replacement"] == "x is not None"

    def test_not_is_none_complex(self):
        findings = check_code("if not foo.bar is None: pass", {"RAB048"})
        f = finding_by_code(findings, "RAB048")
        assert f["fix"]["replacement"] == "foo.bar is not None"

    def test_no_warning_is_none(self):
        assert_no_findings("if x is None: pass")

    def test_no_warning_is_not_none(self):
        assert_no_findings("if x is not None: pass")


class TestRAB049AugmentedAssign:
    def test_add(self):
        findings = check_code("x = x + 1", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x += 1"

    def test_sub(self):
        findings = check_code("x = x - 1", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x -= 1"

    def test_mul(self):
        findings = check_code("x = x * 2", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x *= 2"

    def test_div(self):
        findings = check_code("x = x / 2", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x /= 2"

    def test_floor_div(self):
        findings = check_code("x = x // 2", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x //= 2"

    def test_mod(self):
        findings = check_code("x = x % 2", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x %= 2"

    def test_pow(self):
        findings = check_code("x = x ** 2", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x **= 2"

    def test_bit_and(self):
        findings = check_code("x = x & 1", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x &= 1"

    def test_bit_or(self):
        findings = check_code("x = x | 1", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x |= 1"

    def test_bit_xor(self):
        findings = check_code("x = x ^ 1", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x ^= 1"

    def test_lshift(self):
        findings = check_code("x = x << 1", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x <<= 1"

    def test_rshift(self):
        findings = check_code("x = x >> 1", {"RAB049"})
        f = finding_by_code(findings, "RAB049")
        assert f["fix"]["replacement"] == "x >>= 1"

    def test_no_warning_different_var(self):
        assert_no_findings("x = y + 1", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_multi_target(self):
        assert_no_findings("x = y = z + 1", ignore_codes={"RAB001", "RAB022"})


class TestRAB053IsTrue:
    def test_is_true(self):
        findings = check_code("if x is True: pass", {"RAB053"})
        f = finding_by_code(findings, "RAB053")
        assert f["fix"]["replacement"] == "x"

    def test_is_false(self):
        findings = check_code("if x is False: pass", {"RAB053"})
        f = finding_by_code(findings, "RAB053")
        assert f["fix"]["replacement"] == "not x"

    def test_no_warning_is_none(self):
        assert_no_findings("if x is None: pass")

    def test_no_warning_equality(self):
        assert_no_findings("if x == 1: pass")


class TestRAB050RangeLen:
    def test_range_len(self):
        check_code("for i in range(len(items)):\n    print(items[i])", {"RAB050"})

    def test_no_warning_direct_iter(self):
        assert_no_findings("for item in items:\n    print(item)", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_range_with_arg(self):
        assert_no_findings("for i in range(10):\n    print(i)", ignore_codes={"RAB001", "RAB022"})


class TestRAB051Setdefault:
    def test_setdefault_append(self):
        check_code("d.setdefault(k, []).append(v)", {"RAB051"})

    def test_setdefault_set_add(self):
        check_code("d.setdefault(k, set()).add(v)", {"RAB051"})

    def test_no_warning_normal_append(self):
        assert_no_findings("d[k].append(v)", ignore_codes={"RAB001", "RAB022"})


class TestRAB052TypeIs:
    def test_type_or_type(self):
        findings = check_code("type(x) == int or type(x) == str", {"RAB052"})
        f = finding_by_code(findings, "RAB052")
        assert f["fix"]["replacement"] == "isinstance(x, (int, str))"

    def test_type_or_type_or_type(self):
        findings = check_code("type(x) == int or type(x) == str or type(x) == bool", {"RAB052"})
        f = finding_by_code(findings, "RAB052")
        assert f["fix"]["replacement"] == "isinstance(x, (int, str, bool))"

    def test_no_warning_single_type(self):
        assert_no_findings("type(x) == int", ignore_codes={"RAB001", "RAB006", "RAB022"})


class TestRAB054IfNotAssign:
    def test_if_not_assign(self):
        findings = check_code("if not x:\n    x = 42", {"RAB054"})
        f = finding_by_code(findings, "RAB054")
        assert f["fix"]["replacement"] == "x = x or 42"

    def test_no_warning_else_clause(self):
        assert_no_findings("if not x:\n    x = 42\nelse:\n    pass", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_different_var(self):
        assert_no_findings("if not x:\n    y = 42", ignore_codes={"RAB001", "RAB022"})


class TestRAB055UnusedLoopVar:
    def test_unused_loop_var(self):
        findings = check_code("for i in range(10):\n    print(42)", {"RAB055"})
        f = finding_by_code(findings, "RAB055")
        assert f["fix"]["replacement"] == "_"

    def test_no_warning_used_var(self):
        assert_no_findings("for i in range(10):\n    print(i)", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_underscore(self):
        assert_no_findings("for _ in range(10):\n    print(42)", ignore_codes={"RAB001", "RAB022"})


class TestRAB056NestedWith:
    def test_nested_with(self):
        check_code("with open('a') as f:\n    with open('b') as g:\n        pass", {"RAB056"})

    def test_no_warning_single_with(self):
        assert_no_findings("with open('a') as f:\n    pass", ignore_codes={"RAB001", "RAB022"})


class TestRAB057StartswithOr:
    def test_startswith_or(self):
        findings = check_code("s.startswith('a') or s.startswith('b')", {"RAB057"})
        f = finding_by_code(findings, "RAB057")
        assert "startswith" in f["fix"]["replacement"]
        assert "'a', 'b'" in f["fix"]["replacement"] or '"a", "b"' in f["fix"]["replacement"]

    def test_endswith_or(self):
        findings = check_code("s.endswith('.py') or s.endswith('.pyw')", {"RAB057"})
        f = finding_by_code(findings, "RAB057")
        assert "endswith" in f["fix"]["replacement"]
        assert "'.py'" in f["fix"]["replacement"]

    def test_no_warning_single(self):
        assert_no_findings("s.startswith('a')", ignore_codes={"RAB001", "RAB022"})


class TestRAB058ReturnTernary:
    def test_return_true_cond(self):
        findings = check_code("return True if x > 0 else False", {"RAB058"})
        f = finding_by_code(findings, "RAB058")
        assert f["fix"]["replacement"] == "return x > 0"

    def test_return_false_cond(self):
        findings = check_code("return False if x > 0 else True", {"RAB058"})
        f = finding_by_code(findings, "RAB058")
        assert f["fix"]["replacement"] == "return not x > 0"

    def test_no_warning_normal_return(self):
        assert_no_findings("return x > 0")


class TestRAB059InfiniteWhile:
    def test_while_true(self):
        check_code("while True:\n    print(1)", {"RAB059"})

    def test_no_warning_with_break(self):
        assert_no_findings("while True:\n    if done:\n        break\n    print(1)", ignore_codes={"RAB001", "RAB022"})

    def test_no_warning_condition(self):
        assert_no_findings("while x > 0:\n    print(1)", ignore_codes={"RAB001", "RAB022"})


class TestRAB060SortedSort:
    def test_sorted_sort(self):
        findings = check_code("sorted(x).sort()", {"RAB060"})
        f = finding_by_code(findings, "RAB060")
        assert f["fix"]["replacement"] == "x.sort()"

    def test_sorted_sort_with_key(self):
        findings = check_code("sorted(lst, key=len).sort()", {"RAB060"})
        f = finding_by_code(findings, "RAB060")
        assert f["fix"]["replacement"] == "lst.sort(key=len)"

    def test_no_warning_normal_sort(self):
        assert_no_findings("x.sort()", ignore_codes={"RAB001", "RAB022"})


class TestRAB061WildcardImport:
    def test_wildcard_import(self):
        check_code("from os import *", {"RAB061"})

    def test_no_warning_specific_import(self):
        assert_no_findings("from os import path", ignore_codes={"RAB001", "RAB022"})


class TestRAB062RedundantPass:
    def test_pass_after_docstring(self):
        findings = check_code("def f():\n    \"\"\"doc\"\"\"\n    pass", {"RAB062"})
        f = finding_by_code(findings, "RAB062")
        assert f["fix"] is not None

    def test_no_warning_only_pass(self):
        assert_no_findings("def f():\n    pass", ignore_codes={"RAB001", "RAB022"})


class TestRAB063IsLiteral:
    def test_is_int(self):
        findings = check_code("x is 5", {"RAB063"})
        f = finding_by_code(findings, "RAB063")
        assert f["fix"]["replacement"] == "x == 5"

    def test_is_not_string(self):
        findings = check_code("x is not 'hello'", {"RAB063"})
        f = finding_by_code(findings, "RAB063")
        assert f["fix"]["replacement"] == "x != 'hello'"

    def test_is_float(self):
        findings = check_code("x is 3.14", {"RAB063"})
        f = finding_by_code(findings, "RAB063")
        assert f["fix"]["replacement"] == "x == 3.14"

    def test_no_warning_is_none(self):
        assert_no_findings("x is None")


class TestRAB064InitReturn:
    def test_init_returns_value(self):
        findings = check_code("class Foo:\n    def __init__(self):\n        return 42", {"RAB064"})
        f = finding_by_code(findings, "RAB064")
        assert f["fix"]["replacement"] == "return"

    def test_init_returns_none_ok(self):
        assert_no_findings("class Foo:\n    def __init__(self):\n        return", ignore_codes={"RAB001", "RAB022"})


class TestRAB065DeadCode:
    def test_if_true(self):
        check_code("if True:\n    print(1)", {"RAB065"})

    def test_if_false(self):
        check_code("if False:\n    print(1)", {"RAB065"})

    def test_no_warning_normal_if(self):
        assert_no_findings("if x > 0:\n    print(1)", ignore_codes={"RAB001", "RAB022"})


class TestRAB066DefInLoop:
    def test_def_in_for_loop(self):
        check_code("for x in range(10):\n    def f(): pass", {"RAB066"})

    def test_def_in_while_loop(self):
        check_code("while True:\n    def f(): pass\n    break", {"RAB066"})

    def test_no_warning_def_outside(self):
        assert_no_findings("def f(): pass", ignore_codes={"RAB001", "RAB022"})


class TestRAB067BuiltinShadow:
    def test_shadow_list(self):
        findings = check_code("list = [1, 2, 3]", {"RAB067"})
        f = finding_by_code(findings, "RAB067")
        assert f["fix"]["replacement"] == "list_"

    def test_shadow_dict_function(self):
        check_code("def dict(): pass", {"RAB067"})

    def test_no_warning_normal(self):
        assert_no_findings("x = 42", ignore_codes={"RAB001", "RAB022"})


class TestRAB068RaiseWithoutFrom:
    def test_raise_in_except(self):
        check_code("try:\n    pass\nexcept:\n    raise ValueError('x')", {"RAB068"})

    def test_raise_with_from_ok(self):
        source = "try:\n    pass\nexcept Exception as e:\n    raise ValueError('x') from e"
        assert_no_findings(source, ignore_codes={"RAB001", "RAB022"})

    def test_raise_outside_except_ok(self):
        assert_no_findings("raise ValueError('x')")
