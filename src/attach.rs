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
    base.contains("libpython") || base.starts_with("python")
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

// ── ELF symbol resolution + interpreter version (Linux) ──────────────────────
//
// `_PyRuntime` and `Py_Version` are exported globals, so they live in `.dynsym`
// (present even in stripped binaries). We parse ELF64 by hand to avoid a heavy
// dependency, then apply the module's load bias to turn a link-time vaddr into a
// runtime address in the target.

#[cfg(target_os = "linux")]
mod elf {
    fn u16(b: &[u8], o: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
    }
    fn u32(b: &[u8], o: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
    }
    fn u64(b: &[u8], o: usize) -> Option<u64> {
        Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
    }

    fn is_elf64_le(d: &[u8]) -> bool {
        d.len() > 6 && &d[0..4] == b"\x7fELF" && d[4] == 2 && d[5] == 1
    }

    fn read_cstr(d: &[u8], off: usize) -> Option<&str> {
        let s = d.get(off..)?;
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        std::str::from_utf8(&s[..end]).ok()
    }

    /// Link-time virtual address of an exported symbol, searching `.symtab` and
    /// `.dynsym`.
    pub fn symbol_vaddr(d: &[u8], name: &str) -> Option<u64> {
        if !is_elf64_le(d) {
            return None;
        }
        let e_shoff = u64(d, 0x28)? as usize;
        let e_shentsize = u16(d, 0x3a)? as usize;
        let e_shnum = u16(d, 0x3c)? as usize;
        for i in 0..e_shnum {
            let sh = e_shoff + i * e_shentsize;
            let sh_type = u32(d, sh + 4)?;
            if sh_type != 2 && sh_type != 11 {
                // not SYMTAB / DYNSYM
                continue;
            }
            let sym_off = u64(d, sh + 24)? as usize;
            let sym_size = u64(d, sh + 32)? as usize;
            let link = u32(d, sh + 40)? as usize; // associated string table section
            let entsize = u64(d, sh + 56)? as usize;
            if entsize == 0 {
                continue;
            }
            let str_sh = e_shoff + link * e_shentsize;
            let str_off = u64(d, str_sh + 24)? as usize;
            let count = sym_size / entsize;
            for s in 0..count {
                let so = sym_off + s * entsize;
                let st_name = u32(d, so)? as usize;
                let st_value = u64(d, so + 8)?;
                if st_value == 0 {
                    continue;
                }
                if read_cstr(d, str_off + st_name) == Some(name) {
                    return Some(st_value);
                }
            }
        }
        None
    }

    /// Minimum `p_vaddr` over PT_LOAD segments — the file's link base.
    pub fn min_load_vaddr(d: &[u8]) -> Option<u64> {
        if !is_elf64_le(d) {
            return None;
        }
        let e_phoff = u64(d, 0x20)? as usize;
        let e_phentsize = u16(d, 0x36)? as usize;
        let e_phnum = u16(d, 0x38)? as usize;
        let mut min: Option<u64> = None;
        for i in 0..e_phnum {
            let ph = e_phoff + i * e_phentsize;
            if u32(d, ph)? == 1 {
                // PT_LOAD
                let p_vaddr = u64(d, ph + 16)?;
                min = Some(min.map_or(p_vaddr, |m| m.min(p_vaddr)));
            }
        }
        min
    }
}

#[cfg(target_os = "linux")]
pub struct InterpreterDetails {
    pub module: String,
    pub version_hex: u32,
    pub version: String,
    pub py_runtime_addr: usize,
}

/// The runtime load base for `module_path` in the target: the start of its
/// offset-0 mapping (where the file's first byte lands).
#[cfg(target_os = "linux")]
fn module_load_base(regions: &[MapRegion], module_path: &str) -> Option<usize> {
    let mut with_zero_offset = regions
        .iter()
        .filter(|r| r.path == module_path && r.offset == 0)
        .map(|r| r.start);
    if let Some(base) = with_zero_offset.clone().min() {
        return Some(base);
    }
    let _ = with_zero_offset.next();
    regions
        .iter()
        .filter(|r| r.path == module_path)
        .map(|r| r.start)
        .min()
}

/// Resolve the target's CPython version and `_PyRuntime` address by parsing
/// whichever mapped module exports the interpreter symbols.
#[cfg(target_os = "linux")]
pub fn interpreter_details(pid: i32) -> std::io::Result<InterpreterDetails> {
    let regions = parse_maps(pid)?;
    let mut candidates: Vec<String> = Vec::new();
    for r in &regions {
        if is_python_mapping(&r.path) && !candidates.contains(&r.path) && std::path::Path::new(&r.path).exists() {
            candidates.push(r.path.clone());
        }
    }

    for module in candidates {
        let Ok(data) = std::fs::read(&module) else {
            continue;
        };
        let (Some(rt_vaddr), Some(ver_vaddr)) =
            (elf::symbol_vaddr(&data, "_PyRuntime"), elf::symbol_vaddr(&data, "Py_Version"))
        else {
            continue;
        };
        let base_vaddr = elf::min_load_vaddr(&data).unwrap_or(0);
        let Some(load_base) = module_load_base(&regions, &module) else {
            continue;
        };
        let bias = load_base as i128 - base_vaddr as i128;
        let ver_runtime = (bias + ver_vaddr as i128) as usize;
        let rt_runtime = (bias + rt_vaddr as i128) as usize;

        let bytes = read_mem(pid, ver_runtime, 4)?;
        if bytes.len() < 4 {
            continue;
        }
        let version_hex = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let major = (version_hex >> 24) & 0xff;
        let minor = (version_hex >> 16) & 0xff;
        let micro = (version_hex >> 8) & 0xff;
        // Sanity-check: Python 3.x.
        if major != 3 {
            continue;
        }
        return Ok(InterpreterDetails {
            module,
            version_hex,
            version: format!("{major}.{minor}.{micro}"),
            py_runtime_addr: rt_runtime,
        });
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "could not resolve interpreter symbols (_PyRuntime / Py_Version)",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn interpreter_details(_pid: i32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "interpreter introspection is only implemented on Linux",
    ))
}

// ── remote stack sampling via _Py_DebugOffsets (Linux, CPython 3.12+) ────────
//
// Since 3.12, `_PyRuntime` begins with a `_Py_DebugOffsets` block (cookie
// "xdebugpy") that publishes the struct field offsets needed for out-of-process
// debugging. We read the offsets straight from the target, so the walk adapts to
// the target's exact build instead of hardcoding per-version type layouts. The
// positions below (where each offset lives *inside* `_Py_DebugOffsets`) were
// validated against CPython 3.14 by recovering a known call stack.

#[cfg(target_os = "linux")]
struct DebugOffsets {
    interp_head: usize,
    interp_next: usize,
    threads_head: usize,
    tstate_next: usize,
    tstate_current_frame: usize,
    frame_previous: usize,
    frame_executable: usize,
    code_filename: usize,
    code_name: usize,
}

#[cfg(target_os = "linux")]
fn read_ptr(pid: i32, addr: usize) -> std::io::Result<usize> {
    let b = read_mem(pid, addr, 8)?;
    if b.len() < 8 {
        return Ok(0);
    }
    Ok(u64::from_le_bytes(b[..8].try_into().unwrap()) as usize)
}

#[cfg(target_os = "linux")]
fn read_debug_offsets(pid: i32, runtime_addr: usize) -> std::io::Result<DebugOffsets> {
    let blob = read_mem(pid, runtime_addr, 512)?;
    if blob.len() < 360 || &blob[0..8] != b"xdebugpy" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "_Py_DebugOffsets cookie not found (Python < 3.12?)",
        ));
    }
    let g = |pos: usize| u64::from_le_bytes(blob[pos..pos + 8].try_into().unwrap()) as usize;
    Ok(DebugOffsets {
        interp_head: g(40),
        interp_next: g(64),
        threads_head: g(72),
        tstate_next: g(192),
        tstate_current_frame: g(208),
        frame_previous: g(256),
        frame_executable: g(264),
        code_filename: g(320),
        code_name: g(328),
    })
}

/// Read a CPython compact-ASCII `str` object's text (filenames / function names).
#[cfg(target_os = "linux")]
fn read_pystr(pid: i32, obj: usize) -> Option<String> {
    if obj == 0 {
        return None;
    }
    let hdr = read_mem(pid, obj, 48).ok()?;
    if hdr.len() < 48 {
        return None;
    }
    let length = i64::from_le_bytes(hdr[16..24].try_into().ok()?);
    if length <= 0 || length > 4096 {
        return None;
    }
    // Compact ASCII string data follows the PyASCIIObject header (40 bytes).
    let data = read_mem(pid, obj + 40, length as usize).ok()?;
    let s = String::from_utf8_lossy(&data).into_owned();
    if s.chars().all(|c| c == '\t' || c == '\n' || !c.is_control()) {
        Some(s)
    } else {
        None
    }
}

/// One snapshot of every Python thread's stack in the target. Each stack is a
/// vec of "func\tfile\t0" entries, leaf-first (line numbers are a refinement).
#[cfg(target_os = "linux")]
pub fn sample_stacks(pid: i32) -> std::io::Result<Vec<Vec<String>>> {
    let details = interpreter_details(pid)?;
    let off = read_debug_offsets(pid, details.py_runtime_addr)?;

    let mut stacks = Vec::new();
    let mut interp = read_ptr(pid, details.py_runtime_addr + off.interp_head)?;
    let mut interp_guard = 0;
    while interp != 0 && interp_guard < 64 {
        interp_guard += 1;
        let mut tstate = read_ptr(pid, interp + off.threads_head)?;
        let mut t_guard = 0;
        while tstate != 0 && t_guard < 4096 {
            t_guard += 1;
            let mut frame = read_ptr(pid, tstate + off.tstate_current_frame)?;
            let mut stack = Vec::new();
            let mut depth = 0;
            while frame != 0 && depth < 512 {
                depth += 1;
                let code = read_ptr(pid, frame + off.frame_executable)?;
                if code != 0 {
                    let file = read_pystr(pid, read_ptr(pid, code + off.code_filename)?);
                    let func = read_pystr(pid, read_ptr(pid, code + off.code_name)?);
                    if let (Some(file), Some(func)) = (file, func) {
                        stack.push(format!("{func}\t{file}\t0"));
                    }
                }
                frame = read_ptr(pid, frame + off.frame_previous)?;
            }
            if !stack.is_empty() {
                stacks.push(stack);
            }
            tstate = read_ptr(pid, tstate + off.tstate_next)?;
        }
        interp = read_ptr(pid, interp + off.interp_next)?;
    }
    Ok(stacks)
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

#[cfg(target_os = "linux")]
#[pyfunction]
pub fn attach_sample(py: Python<'_>, pid: i32) -> PyResult<Py<PyAny>> {
    let stacks = sample_stacks(pid)
        .map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("sample failed: {e}")))?;
    let outer = PyList::empty(py);
    for stack in &stacks {
        outer.append(PyList::new(py, stack)?)?;
    }
    Ok(outer.into_any().unbind())
}

#[cfg(not(target_os = "linux"))]
#[pyfunction]
pub fn attach_sample(_py: Python<'_>, _pid: i32) -> PyResult<Py<PyAny>> {
    Err(pyo3::exceptions::PyOSError::new_err(
        "remote sampling is only implemented on Linux",
    ))
}

#[cfg(target_os = "linux")]
#[pyfunction]
pub fn attach_interpreter_info(py: Python<'_>, pid: i32) -> PyResult<Py<PyAny>> {
    let d = interpreter_details(pid)
        .map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("interpreter_details failed: {e}")))?;
    let dict = PyDict::new(py);
    dict.set_item("module", d.module)?;
    dict.set_item("version", d.version)?;
    dict.set_item("version_hex", d.version_hex)?;
    dict.set_item("py_runtime_addr", d.py_runtime_addr)?;
    Ok(dict.into_any().unbind())
}

#[cfg(not(target_os = "linux"))]
#[pyfunction]
pub fn attach_interpreter_info(_py: Python<'_>, _pid: i32) -> PyResult<Py<PyAny>> {
    Err(pyo3::exceptions::PyOSError::new_err(
        "interpreter introspection is only implemented on Linux",
    ))
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
