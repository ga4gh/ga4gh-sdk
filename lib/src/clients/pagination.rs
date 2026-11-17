//! Lazy TES/WES pages and item cursors. No snapshot/exactly-once guarantee.
//!
//! Queries are encoded as described in [`crate::utils::query`], the same as
//! for [`Transport`]; [`Pager::new`] rejects unsupported values.
//!
//! [`TES::pages`] and [`WES::pages`] build a [`Pager`], which fetches pages
//! of the task or run list on demand, with [`Pager::next_page`], or single
//! items across pages, with [`Pager::next_item`]. A [`PageCheckpoint`] saves
//! the position, so that [`Pager::resume`] can continue a listing later, and
//! every [`PageFailure`] carries one.
//!
//! ```no_run
//! use ga4gh_sdk::clients::pagination::PagingOptions;
//! use ga4gh_sdk::clients::wes::WES;
//! use ga4gh_sdk::utils::configuration::Configuration;
//! use std::time::Duration;
//!
//! # async fn example() -> ga4gh_sdk::Result<()> {
//! let config = Configuration::new(url::Url::parse("https://example.org/ga4gh/wes/v1")?);
//! let wes = WES::new(&config).await?;
//! let mut pager = wes.pages(Some(50), PagingOptions::new(Duration::from_secs(60)))?;
//! while let Some(run) = pager.next_item().await? {
//!     println!("{}", run["run_id"]);
//! }
//! # Ok(())
//! # }
//! ```
use super::{
    tes::{models::ListTasksParams, TES},
    wait::ServiceKind,
    wes::WES,
};
use crate::utils::{
    operation::Operation,
    query::query_pairs,
    transport::{response_text, BodyLimits, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet, VecDeque},
    error::Error,
    fmt,
    time::Duration,
};

/// Limits of a listing.
#[derive(Clone, Debug)]
pub struct PagingOptions {
    /// The deadline and cancellation of the whole listing, shared by all of
    /// its page requests.
    pub operation: Operation,
    /// The most page requests (default 1000); then the listing stops without
    /// an error. Must be positive.
    pub max_requests: usize,
    /// The most items in total (default: no limit); the page that exceeds it
    /// is truncated and the listing stops.
    pub max_items: Option<usize>,
    /// The most bytes read of each page's body.
    pub limits: BodyLimits,
}
impl PagingOptions {
    /// Options for a listing that ends `budget` from now, with the defaults
    /// of the fields.
    pub fn new(budget: Duration) -> Self {
        Self {
            operation: Operation::new(budget),
            max_requests: 1000,
            max_items: None,
            limits: Default::default(),
        }
    }
}
/// The position of a listing, saved to resume it with [`Pager::resume`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PageCheckpoint {
    /// The API whose tasks or runs are listed.
    pub service: ServiceKind,
    /// The service's API root, without credentials.
    pub api_root: String,
    /// The query of the listing, without `page_token`.
    pub query: Value,
    /// The token of the next page; `None` when the listing starts at the
    /// service's first page, or after the last page.
    pub next_token: Option<String>,
    /// Whether the last page has been fetched.
    pub finished: bool,
}
/// A page request failed; the listing can resume from `checkpoint`.
#[derive(Debug)]
pub struct PageFailure {
    /// Where to resume: the start of the failed page, or of a partly read
    /// item page (see [`Pager::checkpoint`]).
    pub checkpoint: PageCheckpoint,
    /// What went wrong.
    pub reason: String,
}
impl fmt::Display for PageFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "listing failed; page checkpoint retained: {}",
            self.reason
        )
    }
}
impl Error for PageFailure {}
/// One page of a listing.
#[derive(Debug)]
pub struct Page {
    /// The tasks (TES) or runs (WES) of the page, as the service sent them.
    pub items: Vec<Value>,
    /// Where to resume: after this page, or at its start if it was truncated.
    pub checkpoint: PageCheckpoint,
    /// Whether [`PagingOptions::max_items`] cut the page short.
    pub truncated: bool,
}
/// Fetches the pages of a TES task list or WES run list on demand.
///
/// It stops at the last page or a limit of its [`PagingOptions`], and fails
/// when a page token repeats. Pages are fetched one by one, so a list that
/// changes meanwhile may skip or repeat items.
#[derive(Debug)]
pub struct Pager {
    transport: Transport,
    checkpoint: PageCheckpoint,
    options: PagingOptions,
    seen: HashSet<[u8; 32]>,
    requests: usize,
    items: usize,
    buffer: VecDeque<Value>,
    item_page_start: Option<PageCheckpoint>,
    partial_page: Option<PageCheckpoint>,
    stopped: bool,
}
fn root(transport: &Transport) -> crate::Result<String> {
    let mut root = transport.endpoint("")?;
    let _ = root.set_username("");
    let _ = root.set_password(None);
    Ok(root.into())
}
impl Pager {
    /// Starts a listing of `service` at `transport`'s API root, with the
    /// query parameters of `query`; its `page_token`, if any, is the first
    /// page's. Sends no request.
    ///
    /// # Errors
    ///
    /// Fails if `options.max_requests` is 0, `query` isn't a JSON object, its
    /// `page_token` isn't a string, or it has a value that can't be encoded
    /// as a query parameter.
    pub fn new(
        transport: &Transport,
        service: ServiceKind,
        mut query: Value,
        options: PagingOptions,
    ) -> crate::Result<Self> {
        if options.max_requests == 0 {
            return Err("pagination max_requests must be positive".into());
        }
        let map = query
            .as_object_mut()
            .ok_or("pagination query must be an object")?;
        let token = match map.remove("page_token") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s).filter(|s| !s.is_empty()),
            _ => return Err("page_token must be a string".into()),
        };
        // Reject unsupported values before the first request, not at it.
        query_pairs(&query)?;
        let mut seen = HashSet::new();
        if let Some(token) = &token {
            seen.insert(Sha256::digest(token.as_bytes()).into());
        }
        Ok(Self {
            transport: transport.clone(),
            checkpoint: PageCheckpoint {
                service,
                api_root: root(transport)?,
                query,
                next_token: token,
                finished: false,
            },
            options,
            seen,
            requests: 0,
            items: 0,
            buffer: VecDeque::new(),
            item_page_start: None,
            partial_page: None,
            stopped: false,
        })
    }
    /// Continues a listing from `checkpoint`, for the same `service`, API
    /// root and `query`. Sends no request.
    ///
    /// Pass `query` without `page_token`: the checkpoint's token replaces it,
    /// but a page that leads back to it still counts as a token cycle.
    ///
    /// # Errors
    ///
    /// Fails as [`Self::new`] does, or if `checkpoint` belongs to another
    /// service, API root or query.
    pub fn resume(
        transport: &Transport,
        service: ServiceKind,
        query: Value,
        checkpoint: PageCheckpoint,
        options: PagingOptions,
    ) -> crate::Result<Self> {
        let mut pager = Self::new(transport, service, query, options)?;
        if checkpoint.service != service
            || checkpoint.api_root != pager.checkpoint.api_root
            || checkpoint.query != pager.checkpoint.query
        {
            return Err(
                "pagination checkpoint belongs to a different service/API root/query".into(),
            );
        }
        pager.checkpoint = checkpoint;
        if let Some(token) = &pager.checkpoint.next_token {
            pager.seen.insert(Sha256::digest(token.as_bytes()).into());
        }
        Ok(pager)
    }
    /// At an item cursor's partial page this returns the *start* of that page.
    /// Resuming replays it; callers must reconcile any already-processed items.
    pub fn checkpoint(&self) -> PageCheckpoint {
        self.partial_page
            .as_ref()
            .or({
                if self.buffer.is_empty() {
                    None
                } else {
                    self.item_page_start.as_ref()
                }
            })
            .unwrap_or(&self.checkpoint)
            .clone()
    }
    fn failure(&self, reason: impl ToString) -> PageFailure {
        PageFailure {
            checkpoint: self.checkpoint(),
            reason: reason.to_string(),
        }
    }
    async fn fetch(&mut self) -> Result<Option<Page>, PageFailure> {
        if self.stopped || self.checkpoint.finished {
            return Ok(None);
        }
        if self.requests >= self.options.max_requests
            || self.options.max_items.is_some_and(|max| self.items >= max)
        {
            self.stopped = true;
            return Ok(None);
        }
        let before = self.checkpoint.clone();
        let mut params = self.checkpoint.query.clone();
        if let Some(token) = &self.checkpoint.next_token {
            params["page_token"] = json!(token);
        }
        // `new` checked the query and the token is a string, so this can't fail.
        let pairs = query_pairs(&params).map_err(|e| self.failure(e))?;
        let path = match self.checkpoint.service {
            ServiceKind::Tes => "tasks",
            ServiceKind::Wes => "runs",
        };
        // An unusable path is a failure of this page before any request is
        // counted; `send_authorized` checks it again when it builds the request.
        self.transport.endpoint(path).map_err(|e| self.failure(e))?;
        self.requests += 1;
        let body = self
            .options
            .operation
            .run(async {
                let response = self
                    .transport
                    .send_authorized(
                        reqwest::Method::GET,
                        path,
                        true,
                        |request| Ok(request.query(&pairs)),
                        None,
                    )
                    .await?;
                response_text(response, self.options.limits).await
            })
            .await
            .map_err(|e| self.failure(e))?
            .map_err(|e| self.failure(e))?;
        let body: Value = serde_json::from_str(&body).map_err(|e| self.failure(e))?;
        let mut items = if self.checkpoint.service == ServiceKind::Wes && body.is_array() {
            body.as_array().unwrap().clone()
        } else {
            match body.get(path) {
                Some(Value::Array(items)) => items.clone(),
                None | Some(Value::Null)
                    if self.checkpoint.service == ServiceKind::Wes && body.is_object() =>
                {
                    Vec::new()
                }
                _ => return Err(self.failure("page is missing its items array")),
            }
        };
        let token = match body.get("next_page_token") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()).filter(|s| !s.is_empty()),
            _ => return Err(self.failure("next_page_token must be a string")),
        };
        if token.as_ref().is_some_and(|token| {
            self.seen
                .contains(&<[u8; 32]>::from(Sha256::digest(token.as_bytes())))
        }) {
            return Err(self.failure("continuation token cycle"));
        }
        if let Some(token) = &token {
            self.seen.insert(Sha256::digest(token.as_bytes()).into());
        }
        let remaining = self
            .options
            .max_items
            .map(|max| max.saturating_sub(self.items));
        let truncated = remaining.is_some_and(|max| items.len() > max);
        if let Some(max) = remaining {
            items.truncate(max);
        }
        self.items += items.len();
        self.checkpoint.next_token = token;
        self.checkpoint.finished = self.checkpoint.next_token.is_none();
        if truncated {
            self.partial_page = Some(before);
            self.stopped = true;
        }
        Ok(Some(Page {
            items,
            checkpoint: self.checkpoint(),
            truncated,
        }))
    }
    /// Fetches the next page, or gives `None` after the last page or at a
    /// limit of the options.
    ///
    /// # Errors
    ///
    /// Fails if the request fails, the deadline passes, the operation is
    /// cancelled, the body isn't a page of the API, a page token repeats, or
    /// [`Self::next_item`] has not yet given every item of its page.
    pub async fn next_page(&mut self) -> Result<Option<Page>, PageFailure> {
        if !self.buffer.is_empty() {
            return Err(
                self.failure("cannot request a page while an item page is partially consumed")
            );
        }
        self.fetch().await
    }
    /// Gives the next item, fetching the next page when the current one is
    /// used up; `None` at the end of the listing.
    ///
    /// # Errors
    ///
    /// Fails as [`Self::next_page`] does when it fetches a page.
    pub async fn next_item(&mut self) -> Result<Option<Value>, PageFailure> {
        loop {
            if let Some(item) = self.buffer.pop_front() {
                return Ok(Some(item));
            }
            self.item_page_start = None;
            let before = self.checkpoint.clone();
            match self.fetch().await? {
                Some(page) => {
                    self.buffer = page.items.into();
                    self.item_page_start = Some(before);
                }
                None => return Ok(None),
            }
        }
    }
}
impl TES {
    /// Lists the tasks page by page, with the filters, view and `page_size`
    /// of `params`; its `page_token`, if any, is the first page's.
    ///
    /// # Errors
    ///
    /// Fails as [`Pager::new`] does.
    pub fn pages(
        &self,
        params: Option<ListTasksParams>,
        options: PagingOptions,
    ) -> crate::Result<Pager> {
        Pager::new(
            &self.transport,
            ServiceKind::Tes,
            params
                .map(serde_json::to_value)
                .transpose()?
                .unwrap_or(json!({})),
            options,
        )
    }
}
impl WES {
    /// Lists the runs page by page, with `page_size` runs per page if given.
    ///
    /// # Errors
    ///
    /// Fails as [`Pager::new`] does.
    pub fn pages(&self, page_size: Option<i64>, options: PagingOptions) -> crate::Result<Pager> {
        let query = page_size
            .map(|size| json!({"page_size":size}))
            .unwrap_or(json!({}));
        Pager::new(&self.transport, ServiceKind::Wes, query, options)
    }
}
