//! The `arrow`-feature-gated bridge: a SPARQL SELECT result → a `pyarrow.Table`.
//!
//! sq-lt1ml (gh-910). The whole module compiles only with `--features arrow`;
//! the default wheel carries none of it (and no Arrow dependency). It reuses the merged
//! `sparq-arrow` projection (`to_record_batch`: one `struct<kind, value, datatype,
//! language, direction>` column per variable) and bridges the resulting Arrow
//! `RecordBatch` to Python through the **Arrow C Data Interface** rather than
//! re-serialising: we export the batch as an `FFI_ArrowArrayStream` and expose it on a
//! tiny `_ArrowStream` object implementing the Arrow PyCapsule protocol
//! (`__arrow_c_stream__`), which `pyarrow.table(...)` ingests directly.
//!
//! ## The one `unsafe` block (memory-safety justification)
//!
//! `forbid(unsafe_code)` holds for the whole crate; this module (only) is `deny`-level
//! so the single block below is allowed with an explicit SAFETY note. The block builds a
//! `PyCapsule` whose raw pointer is a leaked `Box<FFI_ArrowArrayStream>`. This is the
//! canonical Arrow producer pattern: the C Data Interface mandates the capsule's pointer
//! be exactly a `*mut FFI_ArrowArrayStream` (so a consumer's
//! `PyCapsule_GetPointer(_, "arrow_array_stream")` yields the struct), which pyo3's safe
//! `PyCapsule::new_with_value` cannot give (it stores an internal box wrapper, not the
//! value pointer). The capsule's C destructor reclaims the box; `FFI_ArrowArrayStream`'s
//! own `Drop` is idempotent w.r.t. the consumer having already moved/released the stream,
//! so the round-trip is leak-free whether or not pyarrow consumed it.

#![allow(unsafe_code)] // see the module docs — one C-Data-Interface capsule block

use std::ffi::CStr;
use std::os::raw::c_void;
use std::ptr::NonNull;

use arrow_array::ffi_stream::FFI_ArrowArrayStream;
use arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader};
use pyo3::exceptions::PyValueError;
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::types::{PyCapsule, PyTuple};
use sparq_engine::QueryResult;

/// The capsule name the Arrow C Data Interface mandates for an exported stream. A
/// consumer calls `PyCapsule_GetPointer(capsule, "arrow_array_stream")` to recover the
/// `FFI_ArrowArrayStream` pointer, so the name must match exactly.
const STREAM_CAPSULE_NAME: &CStr = c"arrow_array_stream";

/// A one-shot Arrow C-stream producer wrapping a single already-materialised
/// `RecordBatch`. It implements the Arrow PyCapsule protocol (`__arrow_c_stream__`), so
/// `pyarrow.table(obj)` / `pyarrow.RecordBatchReader.from_stream(obj)` can ingest it
/// directly. Not exposed in the `sparq` module's public surface — it is an internal
/// transport handle returned only transiently by `query_arrow` before pyarrow consumes
/// it, hence the leading underscore and no `module = "sparq"` doc surface.
#[pyclass(name = "_ArrowStream")]
struct ArrowStream {
    /// `Some` until `__arrow_c_stream__` is called; taking it enforces single-use
    /// (the underlying FFI stream is move-only — a second export would hand out an
    /// already-consumed handle).
    batch: Option<RecordBatch>,
}

#[pymethods]
impl ArrowStream {
    /// The Arrow PyCapsule stream export. `requested_schema` is accepted (per the
    /// protocol) but ignored — we always produce the term-struct schema; a consumer
    /// asking for a different schema gets the native one and may cast on its side.
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_stream__<'py>(
        &mut self,
        py: Python<'py>,
        requested_schema: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyCapsule>> {
        let _ = requested_schema; // schema negotiation is a no-op (see the doc above)
        let batch = self.batch.take().ok_or_else(|| {
            PyValueError::new_err("this Arrow stream has already been consumed")
        })?;
        batch_into_stream_capsule(py, batch)
    }
}

/// Wrap a single `RecordBatch` into an `FFI_ArrowArrayStream` and hand its pointer to a
/// `PyCapsule` named `arrow_array_stream` (the Arrow C Data Interface contract).
fn batch_into_stream_capsule(
    py: Python<'_>,
    batch: RecordBatch,
) -> PyResult<Bound<'_, PyCapsule>> {
    let schema = batch.schema();
    // A `RecordBatchReader` over exactly one batch; `FFI_ArrowArrayStream::new` adopts it
    // and exposes the C-stream get_schema/get_next/release callbacks over it.
    let reader: Box<dyn RecordBatchReader + Send> = Box::new(RecordBatchIterator::new(
        std::iter::once(Ok(batch)),
        schema,
    ));
    let stream = FFI_ArrowArrayStream::new(reader);

    // Leak the stream to a stable heap pointer; the capsule's C destructor reclaims it.
    let boxed = Box::new(stream);
    let ptr = NonNull::new(Box::into_raw(boxed).cast::<c_void>())
        .expect("Box::into_raw is never null");

    // SAFETY (the single unsafe surface of this crate — see the module docs):
    // * `ptr` is a freshly-leaked, non-null, properly-aligned `*mut FFI_ArrowArrayStream`
    //   (just produced by `Box::into_raw`), exactly the type the C Data Interface requires
    //   the `arrow_array_stream` capsule to carry.
    // * `release` (below) is a thread-safe `extern "C"` destructor: it recovers that same
    //   pointer via `PyCapsule_GetPointer` and drops the box once, reclaiming the stream
    //   whether or not a consumer already moved/released it (`FFI_ArrowArrayStream`'s own
    //   `Drop` is idempotent on a released stream). So there is no double free and no leak.
    // * `STREAM_CAPSULE_NAME` is a `'static` C string and matches the name `release` reads
    //   back with, so `PyCapsule_GetPointer` resolves to the same pointer.
    unsafe {
        PyCapsule::new_with_pointer_and_destructor(
            py,
            ptr,
            STREAM_CAPSULE_NAME,
            Some(release_stream_capsule),
        )
    }
}

/// The capsule's C destructor: reclaim the leaked `Box<FFI_ArrowArrayStream>`. Called by
/// CPython when the capsule is garbage-collected (whether or not pyarrow consumed it).
unsafe extern "C" fn release_stream_capsule(capsule: *mut ffi::PyObject) {
    // SAFETY: CPython hands us our own capsule; `PyCapsule_GetPointer` with the name we
    // created it under returns the `*mut FFI_ArrowArrayStream` we leaked (or null, which
    // we guard). Reconstituting the box drops it exactly once.
    let ptr = unsafe { ffi::PyCapsule_GetPointer(capsule, STREAM_CAPSULE_NAME.as_ptr()) };
    if !ptr.is_null() {
        drop(unsafe { Box::from_raw(ptr.cast::<FFI_ArrowArrayStream>()) });
    }
}

/// Build the `RecordBatch` for `result` (via the merged `sparq-arrow` projection), wrap
/// it as an Arrow C-stream, and ingest it into a `pyarrow.Table`.
pub(crate) fn query_result_to_pyarrow_table<'py>(
    py: Python<'py>,
    result: &QueryResult,
) -> PyResult<Bound<'py, PyAny>> {
    // The actual RDF-term → Arrow column mapping is the shared `sparq-arrow` code, so the
    // native and Python exports are byte-identical. Its error (a duplicate SELECT variable,
    // or an Arrow construction failure) maps to a Python `ValueError`.
    let batch: RecordBatch = sparq_arrow::to_record_batch(result)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;

    let stream = Bound::new(py, ArrowStream { batch: Some(batch) })?;

    // `pyarrow.table(obj)` accepts any object exposing `__arrow_c_stream__` (pyarrow >= 14)
    // and returns a `pyarrow.Table`. We import pyarrow lazily so the wheel does not hard-
    // depend on it: only `query_arrow` needs it, and a missing pyarrow yields a clear
    // ImportError rather than a build/runtime dependency.
    let pyarrow = py.import("pyarrow")?;
    let table_fn = pyarrow.getattr("table")?;
    let args = PyTuple::new(py, [stream.into_any()])?;
    table_fn.call1(args)
}
