//! Bulk calls: `BulkPolicy`, `BulkOutcome` and `BulkReport`.
#![allow(clippy::useless_conversion)]

use super::helpers::{count, seconds};
use super::pending::PyPendingResolution;
use crate::utils::convert::to_json;
use ga4gh_sdk::clients::drs::bulk::{BulkDialect, BulkKey, BulkPolicy, BulkReport, ObjectOutcome};
use ga4gh_sdk::clients::drs::resolution::{PendingResolution, ResolutionReference};
use ga4gh_sdk::utils::operation::RetryPolicy;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyMapping, PyString};
use serde_json::Value;

/// Limits of `DRS.get_bulk_objects()` and `DRS.get_bulk_access_urls()`,
/// invalid ones raise `ValueError` there.
#[pyclass(name = "BulkPolicy", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyBulkPolicy {
    pub inner: BulkPolicy,
}

#[pymethods]
impl PyBulkPolicy {
    #[new]
    #[pyo3(signature = (
        chunk_size=100, advertised_limit=None, concurrency=4, max_requests=1000,
        max_split_depth=8, max_attempts=1, initial_delay=0.2, max_delay=10.0
    ))]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chunk_size: i64,
        advertised_limit: Option<i64>,
        concurrency: i64,
        max_requests: i64,
        max_split_depth: i64,
        max_attempts: i64,
        initial_delay: f64,
        max_delay: f64,
    ) -> PyResult<Self> {
        Ok(PyBulkPolicy {
            inner: BulkPolicy {
                chunk_size: count(chunk_size, "chunk_size")?,
                advertised_limit: advertised_limit
                    .map(|limit| count(limit, "advertised_limit"))
                    .transpose()?,
                concurrency: count(concurrency, "concurrency")?,
                max_requests: count(max_requests, "max_requests")?,
                max_split_depth: count(max_split_depth, "max_split_depth")?,
                retry: RetryPolicy {
                    max_attempts: count(max_attempts, "max_attempts")?,
                    initial_delay: seconds(initial_delay, "initial_delay")?,
                    max_delay: seconds(max_delay, "max_delay")?,
                },
                ..Default::default()
            },
        })
    }

    fn with_service_info(&self, service_info: &Bound<'_, PyAny>) -> PyResult<Self> {
        let raw = to_json(service_info, "service info")?;
        let inner = self
            .inner
            .clone()
            .with_service_info(&raw)
            .map_err(|error| PyValueError::new_err(format!("Invalid service info: {error}")))?;
        Ok(PyBulkPolicy { inner })
    }

    /// The most IDs per request.
    #[getter]
    fn chunk_size(&self) -> usize {
        self.inner.chunk_size
    }

    /// The service's most IDs per request, if known, it lowers `chunk_size`.
    #[getter]
    fn advertised_limit(&self) -> Option<usize> {
        self.inner.advertised_limit
    }

    /// The most requests at the same time.
    #[getter]
    fn concurrency(&self) -> usize {
        self.inner.concurrency
    }

    /// The most requests in one call, retries and splits included.
    #[getter]
    fn max_requests(&self) -> usize {
        self.inner.max_requests
    }

    /// request rejection by server as too large (413) is halved.
    #[getter]
    fn max_split_depth(&self) -> usize {
        self.inner.max_split_depth
    }

    /// The most attempts per request, for network errors, 429 and 503.
    #[getter]
    fn max_attempts(&self) -> usize {
        self.inner.retry.max_attempts
    }

    /// The delay before the first retry, in seconds, it doubles up to `max_delay`.
    #[getter]
    fn initial_delay(&self) -> f64 {
        self.inner.retry.initial_delay.as_secs_f64()
    }

    /// The longest delay between retries, in seconds.
    #[getter]
    fn max_delay(&self) -> f64 {
        self.inner.retry.max_delay.as_secs_f64()
    }
}

/// The outcome for one requested ID (or ID pair) of a bulk call.
#[pyclass(name = "BulkOutcome", module = "ga4gh_sdk", subclass)]
pub struct PyBulkOutcome {
    pub object_id: String,
    pub access_id: Option<String>,
    /// `DrsObject`s or `BulkAccessURL`s.
    pub resolved: Vec<PyObject>,
    pub failure_codes: Vec<u16>,
    pub pending: Option<PyPendingResolution>,
    pub errors: Vec<String>,
    pub is_success: bool,
}

impl PyBulkOutcome {
    /// Converts a lib outcome, checking each resolved entry against its model
    pub(super) fn convert<T: IntoPy<PyObject>>(
        py: Python<'_>,
        key: BulkKey,
        outcome: ObjectOutcome<Value>,
        parse: impl Fn(Value) -> Result<T, String>,
    ) -> Self {
        // Rebuilt with the entries that fit the model so that
        // `is_success` is the lib's own `ObjectOutcome::is_success`.
        let mut checked = ObjectOutcome {
            failure_codes: outcome.failure_codes,
            pending: outcome.pending,
            errors: outcome.errors,
            ..Default::default()
        };
        let mut resolved = Vec::new();
        for value in outcome.resolved {
            match parse(value) {
                Ok(item) => {
                    resolved.push(item.into_py(py));
                    checked.resolved.push(Value::Null);
                }
                Err(error) => checked
                    .errors
                    .push(format!("invalid resolved entry: {error}")),
            }
        }
        PyBulkOutcome {
            object_id: key.object_id,
            access_id: key.access_id,
            resolved,
            is_success: checked.is_success(),
            failure_codes: checked.failure_codes,
            pending: checked.pending.map(|pending| PyPendingResolution {
                inner: PendingResolution {
                    reference: ResolutionReference {
                        api_root: pending.api_root,
                        endpoint: pending.endpoint,
                    },
                    retry_after_secs: pending.retry_after_secs,
                },
            }),
            errors: checked.errors,
        }
    }
}

#[pymethods]
impl PyBulkOutcome {
    #[getter]
    fn object_id(&self) -> String {
        self.object_id.clone()
    }

    /// The access ID of the pair, for `get_bulk_access_urls()`, else `None`.
    #[getter]
    fn access_id(&self) -> Option<String> {
        self.access_id.clone()
    }

    /// The `DrsObject`s or `BulkAccessURL`s that the server returned for the ID
    #[getter]
    fn resolved(&self, py: Python<'_>) -> Vec<PyObject> {
        self.resolved
            .iter()
            .map(|item| item.clone_ref(py))
            .collect()
    }

    /// The HTTP status codes that the server gave for the ID, such as 404.
    #[getter]
    fn failure_codes(&self) -> Vec<u16> {
        self.failure_codes.clone()
    }

    /// The pending state, if the server is still preparing the answer.
    #[getter]
    fn pending(&self) -> Option<PyPendingResolution> {
        self.pending.clone()
    }

    /// What went wrong otherwise, such as a missing or duplicate answer.
    #[getter]
    fn errors(&self) -> Vec<String> {
        self.errors.clone()
    }

    /// Whether the server returned exactly one result and nothing else for the ID.
    #[getter]
    fn is_success(&self) -> bool {
        self.is_success
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let object_id = slf.getattr("object_id")?.repr()?;
        let outcome = slf.borrow();
        Ok(format!(
            "{name}(object_id={object_id}, resolved={}, failure_codes={:?}, pending={}, errors={})",
            outcome.resolved.len(),
            outcome.failure_codes,
            if outcome.pending.is_some() {
                "True"
            } else {
                "False"
            },
            outcome.errors.len()
        ))
    }
}

/// The result of a bulk call: one `BulkOutcome` per distinct requested ID
#[pyclass(name = "BulkReport", module = "ga4gh_sdk", subclass)]
pub struct PyBulkReport {
    pub outcomes: Vec<Py<PyBulkOutcome>>,
    pub diagnostics: Vec<String>,
    pub dialects: Vec<String>,
}

impl PyBulkReport {
    pub(super) fn convert<T: IntoPy<PyObject>>(
        py: Python<'_>,
        report: BulkReport<Value>,
        parse: impl Fn(Value) -> Result<T, String> + Copy,
    ) -> PyResult<Self> {
        let outcomes = report
            .outcomes
            .into_iter()
            .map(|(key, outcome)| Py::new(py, PyBulkOutcome::convert(py, key, outcome, parse)))
            .collect::<PyResult<_>>()?;
        Ok(PyBulkReport {
            outcomes,
            diagnostics: report.diagnostics,
            dialects: report
                .dialects
                .into_iter()
                .map(|dialect| match dialect {
                    BulkDialect::Standard => "standard".to_owned(),
                    BulkDialect::Gen3 => "gen3".to_owned(),
                    _ => format!("{dialect:?}"),
                })
                .collect(),
        })
    }
}

#[pymethods]
impl PyBulkReport {
    #[getter]
    fn outcomes(&self, py: Python<'_>) -> Vec<Py<PyBulkOutcome>> {
        self.outcomes
            .iter()
            .map(|item| item.clone_ref(py))
            .collect()
    }

    #[getter]
    fn diagnostics(&self) -> Vec<String> {
        self.diagnostics.clone()
    }

    /// Reply shapes observed across the call, for example `standard` or `gen3`.
    #[getter]
    fn dialects(&self) -> Vec<String> {
        self.dialects.clone()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let report = slf.borrow();
        Ok(format!(
            "{name}(outcomes={}, diagnostics={})",
            report.outcomes.len(),
            report.diagnostics.len()
        ))
    }
}

/// The `(object_id, access_id)` pairs of `DRS.get_bulk_access_urls()`.
pub(super) fn access_pairs(pairs: &Bound<'_, PyAny>) -> PyResult<Vec<BulkKey>> {
    const PAIRS: &str = "pairs must be a mapping or an iterable of (object_id, access_id) pairs";
    if pairs.is_instance_of::<PyString>() {
        return Err(PyValueError::new_err(PAIRS));
    }
    let items = match pairs.downcast::<PyMapping>() {
        Ok(mapping) => mapping.items()?.into_any(),
        Err(_) => pairs.clone(),
    };
    items
        .iter()
        .map_err(|_| PyValueError::new_err(PAIRS))?
        .map(|pair| {
            let (object_id, access_id): (String, String) =
                pair?.extract().map_err(|_| PyValueError::new_err(PAIRS))?;
            Ok(BulkKey {
                object_id,
                access_id: Some(access_id),
            })
        })
        .collect()
}
