//! Out-of-process attach foundation (F4, step 1).
//!
//! The cross-process plumbing for sampling an already-running Python server
//! (py-spy style): read another process's memory and introspect its address
//! space. This step delivers the robust, version-independent pieces:
//!
//! - [`read_mem`] — copy bytes from a target process via `process_vm_readv`.
//! - [`parse_maps`] — parse `/proc/<pid>/maps`.
//! - [`python_info`] — detect whether a pid is a CPython process and locate its
//!   interpreter binary / `libpython` mapping.
//!
//! The remaining (version-fragile) work — resolving `_PyRuntime` / `Py_Version`
//! and walking `_PyInterpreterFrame` per CPython version — builds on this.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

#[derive(Clone, Debug)]
pub struct MapRegion {
    pub start: usize,
    pub end: usize,
    pub perms: String,
    pub offset: usize,
    pub path: String,
}

/// Parse `/proc/<pid>/maps` into regions.
pub fn parse_maps(pid: i32) -> std::io::Result<Vec<MapRegion>> {
    let data = std::fs::read_to_string(format!("/proc/{}/maps", pid))?;
    let mut regions = Vec::new();
    for line in data.lines() {
        // Format: "start-end perms offset dev inode pathname"
        let fields: Vec<&str> = line.splitn(6, char::is_whitespace).collect();
        if fields.len() < 5 {
            continue;
        }
        let Some((s, e)) = fields[0].split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) = (usize::from_str_radix(s, 16), usize::from_str_radix(e, 16)) else {
            continue;
        };
        let offset = usize::from_str_radix(fields[2], 16).unwrap_or(0);
        let path = fields.get(5).map(|p| p.trim().to_string()).unwrap_or_default();
        regions.push(MapRegion {
            start,
            end,
            perms: fields[1].to_string(),
            offset,
            path,
        });
    }
    Ok(regions)
}

/// Read `len` bytes from `addr` in process `pid`.
#[cfg(target_os = "linux")]
pub fn read_mem(pid: i32, addr: usize, len: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let local = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: len,
    };
    let remote = libc::iovec {
        iov_base: addr as *mut libc::c_void,
        iov_len: len,
    };
    // SAFETY: buffers are valid for `len`; the kernel validates the remote range
    // and returns -1/EFAULT for unreadable addresses rather than faulting us.
    let n = unsafe { libc::process_vm_readv(pid, &local, 1, &remote, 1, 0) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    buf.truncate(n as usize);
    Ok(buf)
}

#[cfg(not(target_os = "linux"))]
pub fn read_mem(_pid: i32, _addr: usize, _len: usize) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "remote memory reading is only implemented on Linux",
    ))
}

fn is_python_mapping(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.contains("libpython") || (base.starts_with("python") && base.contains('3'))
}

pub struct PythonInfo {
    pub is_python: bool,
    pub interpreter_path: Option<String>,
    pub base: Option<usize>,
    pub maps_count: usize,
}

/// Inspect a pid: is it CPython, and where is its interpreter image mapped?
pub fn python_info(pid: i32) -> std::io::Result<PythonInfo> {
    let regions = parse_maps(pid)?;
    let maps_count = regions.len();
    // Prefer a libpython mapping; fall back to a python executable mapping.
    let mut chosen: Option<&MapRegion> = None;
    for r in &regions {
        if is_python_mapping(&r.path) {
            if r.path.contains("libpython") {
                chosen = Some(r);
                break;
            }
            if chosen.is_none() {
                chosen = Some(r);
            }
        }
    }
    Ok(PythonInfo {
        is_python: chosen.is_some(),
        interpreter_path: chosen.map(|r| r.path.clone()),
        base: chosen.map(|r| r.start),
        maps_count,
    })
}

// ── Python bindings ──────────────────────────────────────────────────────────

#[pyfunction]
pub fn attach_read_mem(py: Python<'_>, pid: i32, addr: usize, len: usize) -> PyResult<Py<PyBytes>> {
    let bytes = read_mem(pid, addr, len)
        .map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("read_mem failed: {e}")))?;
    Ok(PyBytes::new(py, &bytes).unbind())
}

#[pyfunction]
pub fn attach_maps(py: Python<'_>, pid: i32) -> PyResult<Py<PyAny>> {
    let regions =
        parse_maps(pid).map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("parse_maps failed: {e}")))?;
    let list = PyList::empty(py);
    for r in &regions {
        let d = PyDict::new(py);
        d.set_item("start", r.start)?;
        d.set_item("end", r.end)?;
        d.set_item("perms", &r.perms)?;
        d.set_item("offset", r.offset)?;
        d.set_item("path", &r.path)?;
        list.append(d)?;
    }
    Ok(list.into_any().unbind())
}

#[pyfunction]
pub fn attach_python_info(py: Python<'_>, pid: i32) -> PyResult<Py<PyAny>> {
    let info =
        python_info(pid).map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("python_info failed: {e}")))?;
    let d = PyDict::new(py);
    d.set_item("is_python", info.is_python)?;
    d.set_item("interpreter_path", info.interpreter_path)?;
    d.set_item("base", info.base)?;
    d.set_item("maps_count", info.maps_count)?;
    Ok(d.into_any().unbind())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_own_memory() {
        let data: [u8; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
        let pid = std::process::id() as i32;
        let got = read_mem(pid, data.as_ptr() as usize, data.len()).expect("read self");
        assert_eq!(got, data);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_own_maps() {
        let pid = std::process::id() as i32;
        let regions = parse_maps(pid).expect("maps");
        assert!(!regions.is_empty());
        for r in &regions {
            assert!(r.start < r.end);
            assert_eq!(r.perms.len(), 4); // e.g. "r-xp"
        }
    }
}
