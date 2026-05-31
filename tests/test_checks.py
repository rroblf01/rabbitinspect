from rabbitinspect import analyze_code, apply_fixes


def check_code(source, expected_codes):
    findings = analyze_code(source)
    codes = {f["code"] for f in findings}
    for code in expected_codes:
        assert code in codes, f"Expected {code} not found in {codes}"
    return findings


def assert_no_findings(source, ignore_codes=None):
    if ignore_codes is None:
        ignore_codes = {"RAB001"}
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
        assert_no_findings("for k in d: pass")


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
        assert_no_findings("if x is True: pass")

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
def simple(x):
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
def check(x):
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
        source = "for k in d.keys(): pass"
        findings = analyze_code(source)
        fixes = [f["fix"] for f in findings if f.get("fix")]
        result = apply_fixes(source, fixes)
        assert "for k in d:" in result
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
