//! Sampling profiler core (F1).
//!
//! A background OS thread periodically acquires the GIL and snapshots every
//! Python thread's stack via `sys._current_frames()`. Using the Python-level
//! frame objects (`f_back` / `f_code` / `f_lineno`) keeps this robust across
//! CPython 3.10–3.14 instead of poking at interpreter struct layouts.
//!
//! Resident memory (RSS) is sampled on the same tick (Linux: /proc/self/status;
//! other platforms degrade to no memory timeline for now).

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Hard cap on stored stack samples so a long-running server can't grow the
/// buffer without bound during the F1 spike.
const MAX_SAMPLES: usize = 2_000_000;

struct Span {
    method: String,
    route: String,
    status: i64,
    start_ms: f64,
    end_ms: f64,
}

struct Query {
    ts_ms: f64,
    sql: String,
    duration_ms: f64,
}

struct Samples {
    start: Instant,
    frame_table: Vec<String>,           // interned "func\tfile\tline" entries
    frame_index: FxHashMap<String, u32>,
    stacks: Vec<Vec<u32>>,              // one entry per (thread, tick); leaf-first frame ids
    sample_ts: Vec<f64>,                // ms since start, parallel to `stacks`
    sample_tid: Vec<u64>,               // thread id, parallel to `stacks`
    rss: Vec<(f64, i64)>,               // (ms, bytes)
    spans: Vec<Span>,                   // request spans recorded by web middleware
    queries: Vec<Query>,                // DB queries recorded by instrumentation
    truncated: bool,
}

impl Samples {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            frame_table: Vec::new(),
            frame_index: FxHashMap::default(),
            stacks: Vec::new(),
            sample_ts: Vec::new(),
            sample_tid: Vec::new(),
            rss: Vec::new(),
            spans: Vec::new(),
            queries: Vec::new(),
            truncated: false,
        }
    }

    fn intern(&mut self, key: String) -> u32 {
        if let Some(&id) = self.frame_index.get(&key) {
            return id;
        }
        let id = self.frame_table.len() as u32;
        self.frame_table.push(key.clone());
        self.frame_index.insert(key, id);
        id
    }
}

struct Sampler {
    stop: &'static AtomicBool,
    handle: Option<JoinHandle<()>>,
    shared: &'static Mutex<Samples>,
}

// The sampler thread holds `'static` references into these, so they must
// outlive any thread we spawn. Leaking on start (and rebuilding on the next
// start) keeps the borrow checker happy without unsafe lifetime tricks.
fn slot() -> &'static Mutex<Option<Sampler>> {
    static S: OnceLock<Mutex<Option<Sampler>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "linux")]
fn read_rss() -> Option<i64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: i64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn read_rss() -> Option<i64> {
    None
}

/// Take one snapshot of every Python thread's stack. Errors (e.g. a frame
/// vanishing mid-walk) abort just this tick.
fn sample_once(py: Python<'_>, shared: &Mutex<Samples>, max_depth: usize) -> PyResult<()> {
    let sys = py.import("sys")?;
    let frames = sys.getattr("_current_frames")?.call0()?;
    let dict = frames.cast::<PyDict>()?;

    let mut s = shared.lock().unwrap();
    if s.stacks.len() >= MAX_SAMPLES {
        s.truncated = true;
        return Ok(());
    }
    let now = s.start.elapsed().as_secs_f64() * 1000.0;

    for (tid_obj, frame_obj) in dict.iter() {
        let tid: u64 = tid_obj.extract().unwrap_or(0);
        let mut stack: Vec<u32> = Vec::new();
        let mut cur = frame_obj;
        let mut depth = 0usize;
        while !cur.is_none() && depth < max_depth {
            let code = cur.getattr("f_code")?;
            let filename: String = code.getattr("co_filename")?.extract()?;
            let name: String = code.getattr("co_name")?.extract()?;
            let lineno: i64 = cur.getattr("f_lineno")?.extract().unwrap_or(0);
            // Skip the profiler's own machinery so it doesn't show up as a hotspot.
            if !name.starts_with("<rabbitinspect-perf>") {
                let key = format!("{}\t{}\t{}", name, filename, lineno);
                let id = s.intern(key);
                stack.push(id);
            }
            cur = cur.getattr("f_back")?;
            depth += 1;
        }
        if !stack.is_empty() {
            s.stacks.push(stack);
            s.sample_ts.push(now);
            s.sample_tid.push(tid);
        }
    }

    if let Some(bytes) = read_rss() {
        s.rss.push((now, bytes));
    }
    Ok(())
}

/// Start the background sampler. `interval_ms` is the gap between snapshots;
/// `max_depth` caps stack-walk depth.
#[pyfunction]
#[pyo3(signature = (interval_ms = 5.0, max_depth = 256))]
pub fn perf_start(interval_ms: f64, max_depth: usize) -> PyResult<()> {
    let mut g = slot().lock().unwrap();
    if g.is_some() {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(
            "profiler already running",
        ));
    }

    let shared: &'static Mutex<Samples> = Box::leak(Box::new(Mutex::new(Samples::new())));
    let stop: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
    let interval = Duration::from_secs_f64((interval_ms.max(0.1)) / 1000.0);

    let handle = std::thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        Python::attach(|py| {
            let _ = sample_once(py, shared, max_depth);
        });
        std::thread::sleep(interval);
    });

    *g = Some(Sampler {
        stop,
        handle: Some(handle),
        shared,
    });
    Ok(())
}

/// Stop the sampler and return the collected data as a dict:
/// `{frames, stacks, ts, tids, rss, duration_ms, sample_count, truncated}`.
#[pyfunction]
pub fn perf_stop(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let mut sampler = {
        let mut g = slot().lock().unwrap();
        match g.take() {
            Some(s) => s,
            None => {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "profiler is not running",
                ))
            }
        }
    };

    sampler.stop.store(true, Ordering::Relaxed);
    // Release the GIL while joining: the sampler thread needs the GIL to finish
    // its in-flight tick before it can observe the stop flag and exit.
    if let Some(handle) = sampler.handle.take() {
        py.detach(move || {
            let _ = handle.join();
        });
    }

    let s = sampler.shared.lock().unwrap();
    let duration_ms = s.start.elapsed().as_secs_f64() * 1000.0;

    let frames = PyList::new(py, &s.frame_table)?;
    let stacks = PyList::empty(py);
    for st in &s.stacks {
        stacks.append(PyList::new(py, st)?)?;
    }
    let ts = PyList::new(py, &s.sample_ts)?;
    let tids = PyList::new(py, &s.sample_tid)?;
    let rss = PyList::empty(py);
    for (t, b) in &s.rss {
        rss.append(PyList::new(py, [*t as f64, *b as f64])?)?;
    }
    let spans = PyList::empty(py);
    for sp in &s.spans {
        let d = PyDict::new(py);
        d.set_item("method", &sp.method)?;
        d.set_item("route", &sp.route)?;
        d.set_item("status", sp.status)?;
        d.set_item("start_ms", sp.start_ms)?;
        d.set_item("end_ms", sp.end_ms)?;
        spans.append(d)?;
    }
    let queries = PyList::empty(py);
    for q in &s.queries {
        let d = PyDict::new(py);
        d.set_item("ts_ms", q.ts_ms)?;
        d.set_item("sql", &q.sql)?;
        d.set_item("duration_ms", q.duration_ms)?;
        queries.append(d)?;
    }

    let out = PyDict::new(py);
    out.set_item("frames", frames)?;
    out.set_item("stacks", stacks)?;
    out.set_item("ts", ts)?;
    out.set_item("tids", tids)?;
    out.set_item("rss", rss)?;
    out.set_item("spans", spans)?;
    out.set_item("queries", queries)?;
    out.set_item("duration_ms", duration_ms)?;
    out.set_item("sample_count", s.stacks.len())?;
    out.set_item("truncated", s.truncated)?;
    Ok(out.into_any().unbind())
}

/// Whether a sampler is currently running.
#[pyfunction]
pub fn perf_running() -> bool {
    slot().lock().unwrap().is_some()
}

/// Drop the current sampler WITHOUT joining its thread. Intended for use in a
/// forked child, where the inherited sampler thread no longer exists (joining
/// would hang). Dropping the `JoinHandle` detaches rather than joins.
#[pyfunction]
pub fn perf_reset() {
    let mut g = slot().lock().unwrap();
    if let Some(s) = g.take() {
        s.stop.store(true, Ordering::Relaxed);
        // `s` (and its JoinHandle) drops here — detach, never join.
    }
}

/// Record a database query with its duration. No-op when not running. The
/// timestamp is taken from the sampler clock so queries align with spans.
#[pyfunction]
pub fn perf_record_query(sql: String, duration_ms: f64) {
    let g = slot().lock().unwrap();
    if let Some(s) = g.as_ref() {
        let mut sh = s.shared.lock().unwrap();
        if sh.queries.len() < MAX_SAMPLES {
            let ts_ms = sh.start.elapsed().as_secs_f64() * 1000.0;
            sh.queries.push(Query {
                ts_ms,
                sql,
                duration_ms,
            });
        }
    }
}

/// Milliseconds elapsed since the current run started, or `-1.0` if not running.
/// Web middleware uses this so request spans share the sampler's clock.
#[pyfunction]
pub fn perf_now_ms() -> f64 {
    let g = slot().lock().unwrap();
    match g.as_ref() {
        Some(s) => s.shared.lock().unwrap().start.elapsed().as_secs_f64() * 1000.0,
        None => -1.0,
    }
}

/// Record an HTTP request span. No-op when the profiler is not running.
#[pyfunction]
pub fn perf_record_span(method: String, route: String, status: i64, start_ms: f64, end_ms: f64) {
    let g = slot().lock().unwrap();
    if let Some(s) = g.as_ref() {
        let mut sh = s.shared.lock().unwrap();
        if sh.spans.len() < MAX_SAMPLES {
            sh.spans.push(Span {
                method,
                route,
                status,
                start_ms,
                end_ms,
            });
        }
    }
}
