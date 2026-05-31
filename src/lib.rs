use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

pub mod analyze;
pub mod checks;
pub mod fix;

#[pyfunction]
fn analyze_code(py: Python<'_>, source: &str) -> PyResult<Vec<Py<PyAny>>> {
    let findings = analyze::analyze_source(source);
    let mut result = Vec::with_capacity(findings.len());

    for finding in findings {
        let dict = PyDict::new(py);
        dict.set_item("line", finding.line)?;
        dict.set_item("col", finding.col)?;
        dict.set_item("end_line", finding.end_line)?;
        dict.set_item("end_col", finding.end_col)?;
        dict.set_item("code", &finding.code)?;
        dict.set_item("message", &finding.message)?;

        if let Some(fix) = &finding.fix {
            let fix_dict = PyDict::new(py);
            fix_dict.set_item("start", fix.start)?;
            fix_dict.set_item("end", fix.end)?;
            fix_dict.set_item("replacement", &fix.replacement)?;
            dict.set_item("fix", fix_dict)?;
        }

        result.push(dict.into());
    }

    Ok(result)
}

#[pyfunction]
fn apply_fixes(_py: Python<'_>, source: &str, fixes: Bound<'_, PyList>) -> PyResult<String> {
    let mut rust_fixes = Vec::new();
    for item in fixes.iter() {
        let dict = item.cast::<PyDict>()?;
        let start: usize = dict
            .get_item("start")?
            .ok_or_else(|| PyValueError::new_err("missing 'start' in fix"))?
            .extract()?;
        let end: usize = dict
            .get_item("end")?
            .ok_or_else(|| PyValueError::new_err("missing 'end' in fix"))?
            .extract()?;
        let replacement: String = dict
            .get_item("replacement")?
            .ok_or_else(|| PyValueError::new_err("missing 'replacement' in fix"))?
            .extract()?;
        rust_fixes.push(analyze::Fix {
            start,
            end,
            replacement,
        });
    }

    let refs: Vec<&analyze::Fix> = rust_fixes.iter().collect();
    Ok(fix::apply_fixes(source, &refs))
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(analyze_code, m)?)?;
    m.add_function(wrap_pyfunction!(apply_fixes, m)?)?;
    Ok(())
}
