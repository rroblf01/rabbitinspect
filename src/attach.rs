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

// ── remote stack sampling via _Py_DebugOffsets (Linux, CPython 3.13+) ────────
//
// Since 3.13, `_PyRuntime` begins with a `_Py_DebugOffsets` block (cookie
// "xdebugpy") that publishes the struct field offsets needed for out-of-process
// debugging. We read those offsets straight from the target, so the walk adapts
// to the target's exact build instead of hardcoding per-version *type* layouts.
// The one thing that still moves between versions is *where* each offset lives
// inside the `_Py_DebugOffsets` blob (later sub-structs slide as earlier ones
// gain fields); see `OffsetPositions` for the per-version position tables,
// validated against CPython 3.13 and 3.14 by recovering a known call stack.
// 3.11/3.12 predate this block and are unsupported for sampling.

#[cfg(target_os = "linux")]
struct DebugOffsets {
    interp_head: usize,
    interp_next: usize,
    threads_head: usize,
    tstate_next: usize,
    tstate_current_frame: usize,
    tstate_native_thread_id: usize,
    frame_previous: usize,
    frame_executable: usize,
    frame_instr_ptr: usize,
    code_filename: usize,
    code_name: usize,
    code_linetable: usize,
    code_firstlineno: usize,
    code_code_adaptive: usize,
}

#[cfg(target_os = "linux")]
fn read_ptr(pid: i32, addr: usize) -> std::io::Result<usize> {
    let b = read_mem(pid, addr, 8)?;
    if b.len() < 8 {
        return Ok(0);
    }
    Ok(u64::from_le_bytes(b[..8].try_into().unwrap()) as usize)
}

// Where each field's offset lives *inside* the target's `_Py_DebugOffsets`
// blob. The cookie + `interpreter_state` start at fixed positions across
// versions, but later sub-structs (thread_state / interpreter_frame /
// code_object) slide as earlier ones gain fields. These two tables were
// derived by dumping the blob from live 3.13 and 3.14 interpreters. The
// in-substruct field order is stable, so only the block start moves.
#[cfg(target_os = "linux")]
struct OffsetPositions {
    interp_head: usize,
    interp_next: usize,
    threads_head: usize,
    tstate_next: usize,
    tstate_current_frame: usize,
    tstate_native_thread_id: usize,
    frame_previous: usize,
    frame_executable: usize,
    frame_instr_ptr: usize,
    code_filename: usize,
    code_name: usize,
    code_linetable: usize,
    code_firstlineno: usize,
    code_code_adaptive: usize,
}

// 3.14 layout (also used as the best guess for newer versions).
#[cfg(target_os = "linux")]
const POS_314: OffsetPositions = OffsetPositions {
    interp_head: 40,
    interp_next: 64,
    threads_head: 72,
    tstate_next: 192,
    tstate_current_frame: 208,
    tstate_native_thread_id: 224,
    frame_previous: 256,
    frame_executable: 264,
    frame_instr_ptr: 272,
    code_filename: 320,
    code_name: 328,
    code_linetable: 344,
    code_firstlineno: 352,
    code_code_adaptive: 384,
};

// 3.13 layout: interpreter_state is smaller, so thread_state/frame/code blocks
// sit earlier in the blob.
#[cfg(target_os = "linux")]
const POS_313: OffsetPositions = OffsetPositions {
    interp_head: 40,
    interp_next: 64,
    threads_head: 72,
    tstate_next: 168,
    tstate_current_frame: 184,
    tstate_native_thread_id: 200,
    frame_previous: 232,
    frame_executable: 240,
    frame_instr_ptr: 248,
    code_filename: 280,
    code_name: 288,
    code_linetable: 304,
    code_firstlineno: 312,
    code_code_adaptive: 344,
};

#[cfg(target_os = "linux")]
fn read_debug_offsets(pid: i32, runtime_addr: usize, minor: u32) -> std::io::Result<DebugOffsets> {
    let blob = read_mem(pid, runtime_addr, 512)?;
    if blob.len() < 360 || &blob[0..8] != b"xdebugpy" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "_Py_DebugOffsets cookie not found (Python < 3.13 has no remote-debug offsets)",
        ));
    }
    // 3.13 introduced _Py_DebugOffsets; 3.14 reshaped the sub-structs. Newer
    // versions default to the 3.14 layout (revalidate when 3.15 ships).
    let p = if minor <= 13 { &POS_313 } else { &POS_314 };
    let g = |pos: usize| u64::from_le_bytes(blob[pos..pos + 8].try_into().unwrap()) as usize;
    Ok(DebugOffsets {
        interp_head: g(p.interp_head),
        interp_next: g(p.interp_next),
        threads_head: g(p.threads_head),
        tstate_next: g(p.tstate_next),
        tstate_current_frame: g(p.tstate_current_frame),
        tstate_native_thread_id: g(p.tstate_native_thread_id),
        frame_previous: g(p.frame_previous),
        frame_executable: g(p.frame_executable),
        frame_instr_ptr: g(p.frame_instr_ptr),
        code_filename: g(p.code_filename),
        code_name: g(p.code_name),
        code_linetable: g(p.code_linetable),
        code_firstlineno: g(p.code_firstlineno),
        code_code_adaptive: g(p.code_code_adaptive),
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

// ── PEP 626 line table decoding ──────────────────────────────────────────────
//
// `co_linetable` is a sequence of variable-length entries (one per run of
// instructions). We walk entries, accumulating the line delta, until the entry
// whose instruction range covers `lasti` (the current instruction index). The
// format is the location-table format from CPython's `Objects/locations.md`.

#[cfg(target_os = "linux")]
fn scan_varint(d: &[u8], i: &mut usize) -> u64 {
    let mut b = *d.get(*i).unwrap_or(&0);
    *i += 1;
    let mut val = (b & 0x3f) as u64;
    let mut shift = 6;
    while b & 0x40 != 0 {
        b = *d.get(*i).unwrap_or(&0);
        *i += 1;
        val |= ((b & 0x3f) as u64) << shift;
        shift += 6;
    }
    val
}

#[cfg(target_os = "linux")]
fn scan_svarint(d: &[u8], i: &mut usize) -> i64 {
    let val = scan_varint(d, i);
    if val & 1 != 0 {
        -((val >> 1) as i64)
    } else {
        (val >> 1) as i64
    }
}

/// Map a current-instruction index (`lasti`, in code units) to a source line,
/// using `co_firstlineno` as the base. Returns 0 if the offset is uncovered.
#[cfg(target_os = "linux")]
fn linetable_to_line(linetable: &[u8], firstlineno: i64, lasti: i64) -> i64 {
    let mut line = firstlineno;
    let mut i = 0usize;
    let mut addr: i64 = 0;
    while i < linetable.len() {
        let first = linetable[i];
        i += 1;
        if first & 0x80 == 0 {
            // Not an entry start byte — bail to avoid desync.
            break;
        }
        let code = (first >> 3) & 0x0f;
        let length = (first & 7) as i64 + 1;
        let ldelta = match code {
            15 => 0,            // NONE
            14 => {             // LONG: signed line delta + 3 varints (end line, cols)
                let d = scan_svarint(linetable, &mut i);
                scan_varint(linetable, &mut i);
                scan_varint(linetable, &mut i);
                scan_varint(linetable, &mut i);
                d
            }
            13 => scan_svarint(linetable, &mut i), // NO_COLUMNS
            10..=12 => {        // ONE_LINE0/1/2: delta = code-10, + 2 column bytes
                i += 2;
                (code as i64) - 10
            }
            _ => {              // 0..=9 SHORT: delta 0, + 1 packed column byte
                i += 1;
                0
            }
        };
        line += ldelta;
        if addr <= lasti && lasti < addr + length {
            return if code == 15 { 0 } else { line };
        }
        addr += length;
    }
    line
}

/// Read a frame's current source line by decoding its code object's line table.
#[cfg(target_os = "linux")]
fn frame_line(pid: i32, frame: usize, code: usize, off: &DebugOffsets) -> i64 {
    // co_firstlineno is a 32-bit int field inside the code object.
    let fl = match read_mem(pid, code + off.code_firstlineno, 4) {
        Ok(b) if b.len() >= 4 => i32::from_le_bytes(b[..4].try_into().unwrap()) as i64,
        _ => return 0,
    };
    // instr_ptr points into the inline co_code_adaptive array; lasti is the
    // instruction index (code units, 2 bytes each) from that array's start.
    let instr_ptr = read_ptr(pid, frame + off.frame_instr_ptr).unwrap_or(0);
    let code_start = code + off.code_code_adaptive;
    if instr_ptr < code_start {
        return fl;
    }
    let lasti = ((instr_ptr - code_start) / 2) as i64;
    // co_linetable is a PyBytes object: ob_size at +16, inline data at +32.
    let lt_obj = read_ptr(pid, code + off.code_linetable).unwrap_or(0);
    if lt_obj == 0 {
        return fl;
    }
    let size = match read_mem(pid, lt_obj + 16, 8) {
        Ok(b) if b.len() >= 8 => i64::from_le_bytes(b[..8].try_into().unwrap()),
        _ => return fl,
    };
    if size <= 0 || size > 1_000_000 {
        return fl;
    }
    let linetable = match read_mem(pid, lt_obj + 32, size as usize) {
        Ok(b) => b,
        _ => return fl,
    };
    linetable_to_line(&linetable, fl, lasti)
}

/// Read a thread's OS scheduling state from `/proc/<pid>/task/<tid>/stat`.
/// Returns the single-char state ('R' running/runnable, 'S'/'D' sleeping, …).
/// Used to classify samples as on-CPU vs off-CPU (waiting). Defaults to 'R' when
/// the stat file can't be read (treat as on-CPU rather than hide the work).
#[cfg(target_os = "linux")]
fn read_thread_state(pid: i32, tid: usize) -> char {
    let path = format!("/proc/{}/task/{}/stat", pid, tid);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return 'R';
    };
    // Format: "tid (comm) state ...". comm may contain spaces/parens, so the
    // state is the first non-space char after the final ')'.
    let Some(close) = data.rfind(')') else {
        return 'R';
    };
    data[close + 1..]
        .trim_start()
        .chars()
        .next()
        .unwrap_or('R')
}

/// One snapshot of every Python thread in the target: its OS state char plus a
/// vec of "func\tfile\tline" entries, leaf-first.
#[cfg(target_os = "linux")]
pub fn sample_stacks(pid: i32) -> std::io::Result<Vec<(char, Vec<String>)>> {
    let details = interpreter_details(pid)?;
    let minor = (details.version_hex >> 16) & 0xff;
    let off = read_debug_offsets(pid, details.py_runtime_addr, minor)?;

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
                        let line = frame_line(pid, frame, code, &off);
                        stack.push(format!("{func}\t{file}\t{line}"));
                    }
                }
                frame = read_ptr(pid, frame + off.frame_previous)?;
            }
            if !stack.is_empty() {
                let native_tid = read_ptr(pid, tstate + off.tstate_native_thread_id)?;
                let state = if native_tid != 0 {
                    read_thread_state(pid, native_tid)
                } else {
                    'R'
                };
                stacks.push((state, stack));
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
    for (state, stack) in &stacks {
        let d = PyDict::new(py);
        d.set_item("state", state.to_string())?;
        d.set_item("frames", PyList::new(py, stack)?)?;
        outer.append(d)?;
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
