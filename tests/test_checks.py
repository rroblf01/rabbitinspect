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
