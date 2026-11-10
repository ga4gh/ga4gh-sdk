//! Lazy TES/WES pages and item cursors. No snapshot/exactly-once guarantee.
use super::{
    tes::{models::ListTasksParams, TES},
    wait::ServiceKind,
    wes::WES,
};
use crate::utils::{
    operation::Operation,
    transport::{response_text, BodyLimits, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    error::Error,
    fmt,
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct PagingOptions {
    pub operation: Operation,
    pub max_requests: usize,
    pub max_items: Option<usize>,
    pub limits: BodyLimits,
}
impl PagingOptions {
    pub fn new(budget: Duration) -> Self {
        Self {
            operation: Operation::new(budget),
            max_requests: 1000,
            max_items: None,
            limits: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PageCheckpoint {
    pub service: ServiceKind,
    pub api_root: String,
    pub query: Value,
    pub next_token: Option<String>,
    pub finished: bool,
}
#[derive(Debug)]
pub struct PageFailure {
    pub checkpoint: PageCheckpoint,
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
#[derive(Debug)]
pub struct Page {
    pub items: Vec<Value>,
    pub checkpoint: PageCheckpoint,
    pub truncated: bool,
}
#[derive(Debug)]
pub struct Pager {
    transport: Transport,
    checkpoint: PageCheckpoint,
    options: PagingOptions,
    seen: HashSet<String>,
    requests: usize,
    items: usize,
    buffer: VecDeque<Value>,
    item_page_start: Option<PageCheckpoint>,
    partial_page: Option<PageCheckpoint>,
    stopped: bool,
}
fn root(transport: &Transport) -> Result<String, Box<dyn Error>> {
    let mut root = transport.endpoint("")?;
    let _ = root.set_username("");
    let _ = root.set_password(None);
    Ok(root.into())
}
impl Pager {
    pub fn new(
        transport: &Transport,
        service: ServiceKind,
        mut query: Value,
        options: PagingOptions,
    ) -> Result<Self, Box<dyn Error>> {
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
        let mut seen = HashSet::new();
        if let Some(token) = &token {
            seen.insert(token.clone());
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
    pub fn resume(
        transport: &Transport,
        service: ServiceKind,
        query: Value,
        checkpoint: PageCheckpoint,
        options: PagingOptions,
    ) -> Result<Self, Box<dyn Error>> {
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
            pager.seen.insert(token.clone());
        }
        Ok(pager)
    }
    /// At an item cursor's partial page this returns the *start* of that page.
    /// Resuming replays it; callers must reconcile any already-processed items.
    pub fn checkpoint(&self) -> PageCheckpoint {
        self.partial_page
            .as_ref()
            .or_else(|| {
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
        let mut pairs = Vec::new();
        for (key, value) in params.as_object().unwrap() {
            let values = match value {
                Value::Array(values) => values.clone(),
                v => vec![v.clone()],
            };
            for value in values {
                if value.is_null() {
                    continue;
                }
                let text = match value {
                    Value::String(s) => s,
                    v => v.to_string(),
                };
                pairs.push((key.clone(), text));
            }
        }
        let path = match self.checkpoint.service {
            ServiceKind::Tes => "tasks",
            ServiceKind::Wes => "runs",
        };
        let request = self
            .transport
            .request_builder(reqwest::Method::GET, path)
            .map_err(|e| self.failure(e))?
            .query(&pairs);
        self.requests += 1;
        let body = self
            .options
            .operation
            .run(async { response_text(request.send().await?, self.options.limits).await })
            .await
            .map_err(|e| self.failure(e))?
            .map_err(|e| self.failure(e))?;
        let body: Value = serde_json::from_str(&body).map_err(|e| self.failure(e))?;
        let mut items = if self.checkpoint.service == ServiceKind::Wes && body.is_array() {
            body.as_array().unwrap().clone()
        } else {
            body.get(path)
                .and_then(Value::as_array)
                .ok_or_else(|| self.failure("page is missing its items array"))?
                .clone()
        };
        let token = match body.get("next_page_token") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()).filter(|s| !s.is_empty()),
            _ => return Err(self.failure("next_page_token must be a string")),
        };
        if token
            .as_ref()
            .is_some_and(|token| self.seen.contains(token))
        {
            return Err(self.failure("continuation token cycle"));
        }
        if let Some(token) = &token {
            self.seen.insert(token.clone());
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
    pub async fn next_page(&mut self) -> Result<Option<Page>, PageFailure> {
        if !self.buffer.is_empty() {
            return Err(
                self.failure("cannot request a page while an item page is partially consumed")
            );
        }
        self.fetch().await
    }
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
    pub fn pages(
        &self,
        params: Option<ListTasksParams>,
        options: PagingOptions,
    ) -> Result<Pager, Box<dyn Error>> {
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
    pub fn pages(
        &self,
        page_size: Option<i64>,
        options: PagingOptions,
    ) -> Result<Pager, Box<dyn Error>> {
        let query = page_size
            .map(|size| json!({"page_size":size}))
            .unwrap_or(json!({}));
        Pager::new(&self.transport, ServiceKind::Wes, query, options)
    }
}
