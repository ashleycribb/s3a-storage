//! Python bindings over the reference engine (TILE-SPEC reader + query).
//! Install: `pip install ./bindings/python` (builds with maturin). See bindings/python/README.md.
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use std::path::Path;

/// Conformance report for an archive, as JSON text (conformance/README.md).
#[pyfunction]
fn read_report_json(path: &str) -> String {
    s3a_spec_vectors::conformance::read_report(Path::new(path)).to_string()
}

/// Merkle inclusion-proof check for a conformance `merkle` case given as JSON text.
#[pyfunction]
fn merkle_report_json(case_json: &str) -> PyResult<String> {
    let v: serde_json::Value =
        serde_json::from_str(case_json).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(s3a_spec_vectors::conformance::merkle_report(&v).to_string())
}

/// A read-only, memory-mapped S3A archive.
#[pyclass(module = "s3a._native")]
struct Archive {
    reader: s3a_engine::MmapReader,
}

#[pymethods]
impl Archive {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        s3a_engine::MmapReader::open(path)
            .map(|reader| Archive { reader })
            .map_err(|e| PyOSError::new_err(format!("{path}: {e}")))
    }

    #[getter]
    fn tile_count(&self) -> u32 {
        self.reader.tile_count()
    }

    #[getter]
    fn generation(&self) -> u64 {
        self.reader.generation()
    }

    /// Live telemetry records with `min_ts <= timestamp <= max_ts`, as
    /// `(timestamp, sensor_id, metric_id, value, flags)` tuples.
    #[pyo3(signature = (min_ts=0, max_ts=u64::MAX))]
    fn query_telemetry(&self, min_ts: u64, max_ts: u64) -> Vec<(u64, u32, u32, f64, u64)> {
        s3a_engine::QuerySieve::new(&self.reader)
            .query_telemetry(min_ts, max_ts, None, None)
            .iter()
            .map(|r| (r.timestamp, r.sensor_id, r.metric_id, r.value, r.flags))
            .collect()
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(read_report_json, m)?)?;
    m.add_function(wrap_pyfunction!(merkle_report_json, m)?)?;
    m.add_class::<Archive>()?;
    Ok(())
}
