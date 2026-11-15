//! One page of TRS tools and its response headers.

use super::tools::PyTool;
use ga4gh_sdk::clients::trs::responses::ToolPage;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use reqwest::header::HeaderMap;

/// One page of tools from `TRS.list_tools()` or `TRS.next_tools_page()`.
///
/// `next_page` is the URL of the next page that the server sent, or `None` on the last
/// page; pass it to `TRS.next_tools_page()`.
#[pyclass(name = "ToolPage", module = "ga4gh_sdk", subclass)]
pub struct PyToolPage {
    pub tools: Vec<PyTool>,
    /// The response headers as received.
    pub headers: HeaderMap,
}

impl PyToolPage {
    /// Wraps a page of the Rust client.
    pub fn new(page: ToolPage) -> PyResult<Self> {
        let tools = page
            .tools
            .into_iter()
            .map(PyTool::new)
            .collect::<PyResult<_>>()?;
        Ok(Self {
            tools,
            headers: page.headers,
        })
    }

    /// The first value of a header, as `ToolPage::next_page` reads it, or `None` if it is
    /// missing, empty or not text.
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }
}

#[pymethods]
impl PyToolPage {
    #[getter]
    pub fn tools(&self) -> Vec<PyTool> {
        self.tools.clone()
    }

    /// The URL of the next page (`next_page` header), or `None`.
    #[getter]
    pub fn next_page(&self) -> Option<String> {
        self.header("next_page")
    }

    /// The URL of the last page (`last_page` header), or `None`.
    #[getter]
    pub fn last_page(&self) -> Option<String> {
        self.header("last_page")
    }

    /// The URL of this page (`self_link` header), or `None`.
    #[getter]
    pub fn self_link(&self) -> Option<String> {
        self.header("self_link")
    }

    /// The offset of this page (`current_offset` header), as the server sent it, or `None`.
    #[getter]
    pub fn current_offset(&self) -> Option<String> {
        self.header("current_offset")
    }

    /// The page size (`current_limit` header), as the server sent it, or `None`.
    #[getter]
    pub fn current_limit(&self) -> Option<String> {
        self.header("current_limit")
    }

    /// All response headers as a new `dict`, by lowercase name. Repeated headers are joined
    /// with `", "`; headers whose values aren't text are left out.
    #[pyo3(name = "headers")]
    pub fn headers_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for name in self.headers.keys() {
            let values: Vec<&str> = self
                .headers
                .get_all(name)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .collect();
            if !values.is_empty() {
                dict.set_item(name.as_str(), values.join(", "))?;
            }
        }
        Ok(dict)
    }

    fn __len__(&self) -> usize {
        self.tools.len()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let this = slf.borrow();
        let next_page = this.next_page().is_some();
        Ok(format!(
            "{name}(tools={}, next_page={})",
            this.tools.len(),
            if next_page { "True" } else { "False" }
        ))
    }
}
