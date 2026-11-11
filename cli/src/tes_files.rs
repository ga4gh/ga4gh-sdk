//! Lists, shows and saves the logs and output files of a TES task.
//!
//! The commands read the task's `FULL` view as JSON, without the SDK models,
//! so that an incomplete task, which lacks logs or output fields, can still be
//! inspected. Each log and output reference gets a selector, such as
//! `stdout:0:0` or `file:0:1`, that names it for `tes log` and `tes save`.
//!
//! Downloads send no credentials from the CLI configuration: output URLs can
//! point to any host. S3 requests use the AWS environment variables only.
//! Every file is written to a temporary file in the destination directory and
//! moved to its name only once all its bytes have arrived.

use crate::render::{needs_escape, printable};
use crate::CommandOutput;
use futures_util::StreamExt;
use ga4gh_sdk::utils::configuration::Configuration;
use serde_json::Value;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use url::Url;

/// Task states after which the logs and outputs no longer change.
const FINAL_STATES: [&str; 5] = [
    "COMPLETE",
    "EXECUTOR_ERROR",
    "SYSTEM_ERROR",
    "CANCELED",
    "PREEMPTED",
];

/// The URL schemes that `tes save` can download.
const SUPPORTED_SCHEMES: &str = "http, https and s3";

/// What a task holds under one selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Content {
    /// Text that the task response contains: an executor's `stdout` or
    /// `stderr`, or an attempt's `system_logs` as lines.
    Inline(String),
    /// A file in storage, referenced by URL.
    File {
        url: String,
        path: Option<String>,
        size: Option<u64>,
        /// Whether the reference names a directory or a wildcard pattern,
        /// which can't be downloaded as one file.
        is_directory: bool,
    },
}

/// One log or output reference of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub selector: String,
    pub description: String,
    pub content: Content,
}

/// The logs and output references found in a task, and notes on what is
/// missing.
#[derive(Debug, Default)]
pub(crate) struct Catalog {
    pub items: Vec<Item>,
    pub notes: Vec<String>,
}

impl Catalog {
    /// Collects the logs and output references from a task's `FULL` JSON.
    pub fn from_task(task: &Value) -> Self {
        let mut catalog = Catalog::default();
        let state = task.get("state").and_then(Value::as_str);
        match state {
            Some(state) if FINAL_STATES.contains(&state) => {}
            Some(state) => catalog.notes.push(format!(
                "The task is in state {state}; its logs and outputs may be incomplete."
            )),
            None => catalog
                .notes
                .push("The task has no state; its logs and outputs may be incomplete.".into()),
        }
        catalog.add_declared_outputs(task.get("outputs"));
        catalog.add_logs(task.get("logs"));
        catalog
    }

    fn add_declared_outputs(&mut self, outputs: Option<&Value>) {
        let Some(outputs) = outputs.and_then(Value::as_array) else {
            if outputs.is_some_and(|value| !value.is_null()) {
                self.notes.push("The task's outputs are not a list.".into());
            }
            return;
        };
        for (index, output) in outputs.iter().enumerate() {
            let selector = format!("output:{index}");
            let Some(url) = output.get("url").and_then(Value::as_str) else {
                self.notes.push(format!("{selector} has no URL."));
                continue;
            };
            let path = output.get("path").and_then(Value::as_str);
            let is_directory = output.get("type").and_then(Value::as_str) == Some("DIRECTORY")
                || output.get("path_prefix").is_some_and(|v| !v.is_null())
                || path.is_some_and(|path| path.contains('*'));
            let kind = if is_directory {
                "declared output directory"
            } else {
                "declared output file"
            };
            self.items.push(Item {
                selector,
                description: kind.into(),
                content: Content::File {
                    url: url.into(),
                    path: path.map(Into::into),
                    size: None,
                    is_directory,
                },
            });
        }
    }

    fn add_logs(&mut self, logs: Option<&Value>) {
        let attempts = match logs.and_then(Value::as_array) {
            Some(attempts) if !attempts.is_empty() => attempts,
            _ => {
                self.notes.push("The task has no logs yet.".into());
                return;
            }
        };
        for (attempt, log) in attempts.iter().enumerate() {
            match log.get("logs").and_then(Value::as_array) {
                Some(executors) if !executors.is_empty() => {
                    for (executor, executor_log) in executors.iter().enumerate() {
                        self.add_executor_log(attempt, executor, executor_log);
                    }
                }
                _ => self
                    .notes
                    .push(format!("Attempt {attempt} has no executor logs.")),
            }
            match log.get("system_logs").and_then(Value::as_array) {
                Some(lines) => {
                    let text: String = lines
                        .iter()
                        .map(|line| match line {
                            Value::String(line) => format!("{line}\n"),
                            other => format!("{other}\n"),
                        })
                        .collect();
                    self.items.push(Item {
                        selector: format!("system:{attempt}"),
                        description: format!("system logs, {} lines", lines.len()),
                        content: Content::Inline(text),
                    });
                }
                None => self
                    .notes
                    .push(format!("Attempt {attempt} has no system logs.")),
            }
            if let Some(files) = log.get("outputs").and_then(Value::as_array) {
                for (index, file) in files.iter().enumerate() {
                    self.add_file_log(attempt, index, file);
                }
            }
        }
    }

    fn add_executor_log(&mut self, attempt: usize, executor: usize, log: &Value) {
        let exit_code = log
            .get("exit_code")
            .map_or_else(|| "none".to_string(), |code| printable(&code.to_string()));
        for stream in ["stdout", "stderr"] {
            let selector = format!("{stream}:{attempt}:{executor}");
            match log.get(stream).and_then(Value::as_str) {
                Some(text) => self.items.push(Item {
                    selector,
                    description: format!(
                        "executor {stream}, {} bytes, exit code {exit_code}",
                        text.len()
                    ),
                    content: Content::Inline(text.into()),
                }),
                None => self
                    .notes
                    .push(format!("{selector} is not in the task response.")),
            }
        }
    }

    fn add_file_log(&mut self, attempt: usize, index: usize, file: &Value) {
        let selector = format!("file:{attempt}:{index}");
        let Some(url) = file.get("url").and_then(Value::as_str) else {
            self.notes.push(format!("{selector} has no URL."));
            return;
        };
        // TES 1.x encodes `size_bytes` as a string; some servers send a number.
        let size = match file.get("size_bytes") {
            Some(Value::String(size)) => size.parse().ok(),
            Some(Value::Number(size)) => size.as_u64(),
            _ => None,
        };
        if size.is_none() {
            self.notes
                .push(format!("{selector} has no valid size_bytes."));
        }
        self.items.push(Item {
            selector,
            description: "output file".into(),
            content: Content::File {
                url: url.into(),
                path: file.get("path").and_then(Value::as_str).map(Into::into),
                size,
                is_directory: false,
            },
        });
    }

    /// Finds the item for a selector, or explains why there is none.
    pub fn find(&self, selector: &str) -> Result<&Item, InputError> {
        let normalized = parse_selector(selector)?;
        self.items
            .iter()
            .find(|item| item.selector == normalized)
            .ok_or_else(|| {
                InputError(format!(
                    "The task has no {normalized}; run `tes files` to list what it has"
                ))
            })
    }

    /// Formats the items as a table, with credentials and query strings
    /// removed from URLs.
    pub fn table(&self) -> String {
        let mut table = format!("{:<14} {:<10} {}\n", "SELECTOR", "SIZE", "DESCRIPTION");
        for item in &self.items {
            let (size, detail) = match &item.content {
                Content::Inline(text) => (text.len().to_string(), String::new()),
                Content::File {
                    url, path, size, ..
                } => {
                    let path = path
                        .as_deref()
                        .map(|path| format!(" (path {})", printable(path)))
                        .unwrap_or_default();
                    (
                        size.map_or_else(|| "-".to_string(), |size| size.to_string()),
                        format!(": {}{path}", display_url(url)),
                    )
                }
            };
            table.push_str(&format!(
                "{:<14} {:<10} {}{detail}\n",
                item.selector,
                size,
                printable(&item.description)
            ));
        }
        table
    }
}

/// A problem with the command's input, reported before any download.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InputError(pub String);

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InputError {}

/// Checks a selector's form and returns it without leading zeros.
pub(crate) fn parse_selector(selector: &str) -> Result<String, InputError> {
    let invalid = || {
        InputError(format!(
            "Invalid selector '{}': expected stdout:ATTEMPT:EXECUTOR, \
             stderr:ATTEMPT:EXECUTOR, system:ATTEMPT, output:INDEX or file:ATTEMPT:INDEX",
            printable(selector)
        ))
    };
    let mut parts = selector.split(':');
    let kind = parts.next().unwrap_or_default();
    let numbers = parts
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            part.parse::<usize>().map_err(|_| invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let expected = match kind {
        "stdout" | "stderr" | "file" => 2,
        "system" | "output" => 1,
        _ => return Err(invalid()),
    };
    if numbers.len() != expected {
        return Err(invalid());
    }
    let numbers: Vec<String> = numbers.iter().map(ToString::to_string).collect();
    Ok(format!("{kind}:{}", numbers.join(":")))
}

/// Shows a URL without credentials, query or fragment, which may hold
/// secrets such as the signature of a pre-signed URL.
pub(crate) fn display_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut parsed) => {
            let hidden = parsed.query().is_some() || parsed.fragment().is_some();
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            let mut shown = printable(parsed.as_str());
            if hidden {
                shown.push_str("?...");
            }
            shown
        }
        Err(_) => "(malformed URL)".into(),
    }
}

/// Escapes a log for text output line by line, keeping its line breaks.
pub(crate) fn printable_lines(text: &str) -> String {
    let mut output: String = text
        .split('\n')
        .map(printable)
        .collect::<Vec<_>>()
        .join("\n");
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

/// Checks a file name chosen with `--name`. It must be a plain name in the
/// destination directory.
pub(crate) fn check_user_name(name: &str) -> Result<String, InputError> {
    if Path::new(name).is_absolute() || name.starts_with('/') || name.starts_with('\\') {
        return Err(InputError(format!(
            "--name '{}' is an absolute path; give a file name, and the directory with --dir",
            printable(name)
        )));
    }
    if name.split(['/', '\\']).any(|part| part == "..") {
        return Err(InputError(format!(
            "--name '{}' contains '..'; give a file name, and the directory with --dir",
            printable(name)
        )));
    }
    if !is_plain_name(name) {
        return Err(InputError(format!(
            "--name '{}' is not a plain file name; give the directory with --dir",
            printable(name)
        )));
    }
    Ok(name.into())
}

/// Derives a file name from an item: the last part of the task's path for a
/// file, else of its URL. Server-provided names that aren't plain file names
/// are refused, never cleaned up.
pub(crate) fn default_name(item: &Item) -> Result<String, InputError> {
    let (url, path) = match &item.content {
        Content::Inline(_) => return Ok(format!("{}.txt", item.selector.replace(':', "-"))),
        Content::File { url, path, .. } => (url, path),
    };
    let from_path = path
        .as_deref()
        .and_then(|path| path.trim_end_matches('/').rsplit('/').next())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string);
    let from_url = || {
        let url = Url::parse(url).ok()?;
        let last = url
            .path_segments()?
            .rev()
            .find(|segment| !segment.is_empty())?;
        percent_decode(last)
    };
    let name = from_path.or_else(from_url).ok_or_else(|| {
        InputError(format!(
            "{} has no file name; choose one with --name",
            item.selector
        ))
    })?;
    if is_plain_name(&name) {
        Ok(name)
    } else {
        Err(InputError(format!(
            "The server's file name '{}' for {} is not a plain file name; choose one with --name",
            printable(&name),
            item.selector
        )))
    }
}

fn percent_decode(segment: &str) -> Option<String> {
    percent_encoding::percent_decode_str(segment)
        .decode_utf8()
        .ok()
        .map(|decoded| decoded.into_owned())
}

/// A plain file name has no path separators, isn't `.` or `..`, and has no
/// control or bidirectional formatting characters.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(needs_escape)
        && !(cfg!(windows) && name.contains([':', '*', '?', '"', '<', '>', '|']))
}

/// A selected item and the name it is saved under.
struct Planned<'a> {
    item: &'a Item,
    name: String,
}

/// Plans what to save and checks every name before any download starts.
fn plan<'a>(
    catalog: &'a Catalog,
    selectors: &[String],
    name: Option<&str>,
    dir: &Path,
    force: bool,
) -> Result<Vec<Planned<'a>>, InputError> {
    if name.is_some() && selectors.len() != 1 {
        return Err(InputError("--name needs exactly one selector".into()));
    }
    let mut planned: Vec<Planned> = Vec::new();
    for selector in selectors {
        let item = catalog.find(selector)?;
        if let Content::File {
            is_directory: true, ..
        } = item.content
        {
            return Err(InputError(format!(
                "{} is a directory or a pattern and can't be saved as one file; \
                 save the matching file:ATTEMPT:INDEX entries instead",
                item.selector
            )));
        }
        let name = match name {
            Some(name) => check_user_name(name)?,
            None => default_name(item)?,
        };
        if planned
            .iter()
            .any(|other| other.item.selector == item.selector)
        {
            return Err(InputError(format!(
                "{} is selected more than once",
                item.selector
            )));
        }
        if planned.iter().any(|other| other.name == name) {
            return Err(InputError(format!(
                "Two selections would be saved as '{}'; save them separately with --name",
                printable(&name)
            )));
        }
        if !force && dir.join(&name).symlink_metadata().is_ok() {
            return Err(InputError(format!(
                "{} already exists; use --force to replace it",
                printable(&dir.join(&name).display().to_string())
            )));
        }
        planned.push(Planned { item, name });
    }
    Ok(planned)
}

/// Shows an inline log as text.
pub(crate) fn show(catalog: &Catalog, selector: &str) -> Result<CommandOutput, InputError> {
    let item = catalog.find(selector)?;
    match &item.content {
        Content::Inline(text) => Ok(CommandOutput::success(printable_lines(text))),
        Content::File { .. } => Err(InputError(format!(
            "{} is a file in storage; download it with `tes save`",
            item.selector
        ))),
    }
}

/// Saves the selected items in `dir`, in order, and stops at the first error.
pub(crate) async fn save(
    catalog: &Catalog,
    selectors: &[String],
    name: Option<&str>,
    dir: &Path,
    force: bool,
    config: &Configuration,
) -> Result<CommandOutput, InputError> {
    let planned = plan(catalog, selectors, name, dir, force)?;
    if let Err(error) = std::fs::create_dir_all(dir) {
        return Ok(CommandOutput::failure(format!(
            "Can't create {}: {error}",
            dir.display()
        )));
    }
    let mut stdout = String::new();
    for entry in planned {
        let destination = dir.join(&entry.name);
        match save_one(entry.item, &destination, force, config).await {
            Ok(bytes) => stdout.push_str(&format!(
                "Saved {} to {} ({bytes} bytes)\n",
                entry.item.selector,
                printable(&destination.display().to_string())
            )),
            Err(error) => {
                return Ok(CommandOutput {
                    stdout,
                    stderr: format!(
                        "{}\n",
                        printable(&format!("{}: {error}", entry.item.selector))
                    ),
                    exit_code: ExitCode::FAILURE,
                })
            }
        }
    }
    Ok(CommandOutput::success(stdout))
}

/// Writes one item to a temporary file next to its destination, then moves it
/// there. The temporary file is removed if anything fails.
async fn save_one(
    item: &Item,
    destination: &Path,
    force: bool,
    config: &Configuration,
) -> anyhow::Result<u64> {
    let dir = destination.parent().unwrap_or(Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(".ga4gh-cli-").suffix(".part");
    // Saved files get the usual permissions under the umask, not 0600.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let mut temp = builder
        .tempfile_in(dir)
        .map_err(|error| anyhow::anyhow!("Can't create a file in {}: {error}", dir.display()))?;
    let written = match &item.content {
        Content::Inline(text) => {
            temp.write_all(text.as_bytes())?;
            text.len() as u64
        }
        Content::File { url, size, .. } => {
            let written = download(url, temp.as_file_mut(), config).await?;
            if let Some(size) = size.filter(|size| *size != written) {
                anyhow::bail!(
                    "Received {written} bytes, but the task reports {size}; the file was not saved"
                );
            }
            written
        }
    };
    temp.as_file().sync_all()?;
    let result = if force {
        temp.persist(destination).map(|_| ())
    } else {
        temp.persist_noclobber(destination).map(|_| ())
    };
    result.map_err(|error| {
        anyhow::anyhow!("Can't save {}: {}", destination.display(), error.error)
    })?;
    Ok(written)
}

/// Streams the object at `url` into `file` and returns the number of bytes.
async fn download(
    url: &str,
    file: &mut std::fs::File,
    config: &Configuration,
) -> anyhow::Result<u64> {
    let parsed = Url::parse(url).map_err(|error| anyhow::anyhow!("Malformed URL: {error}"))?;
    match parsed.scheme() {
        "http" | "https" => download_http(&parsed, file, config).await,
        "s3" => download_s3(&parsed, file, config).await,
        scheme => anyhow::bail!(
            "Unsupported URL scheme '{}'; supported: {SUPPORTED_SCHEMES}",
            printable(scheme)
        ),
    }
}

async fn download_http(
    url: &Url,
    file: &mut std::fs::File,
    config: &Configuration,
) -> anyhow::Result<u64> {
    let shown = display_url(url.as_str());
    // A plain client: output hosts get no credentials from the configuration.
    let client = reqwest::Client::builder()
        .connect_timeout(config.timeouts.connect)
        .build()
        .map_err(|error| anyhow::anyhow!("Can't create an HTTP client: {error}"))?;
    let response =
        client.get(url.clone()).send().await.map_err(|error| {
            anyhow::anyhow!("Request to {shown} failed: {}", error.without_url())
        })?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("{shown} returned HTTP {status}");
    }
    let expected = response.content_length();
    let mut stream = Box::pin(response.bytes_stream());
    let mut written = 0u64;
    while let Some(chunk) =
        next_chunk(&mut stream, config.timeouts.request, &shown, written).await?
    {
        let chunk = chunk.map_err(|error| {
            anyhow::anyhow!(
                "Download from {shown} was interrupted after {written} bytes: {}",
                error.without_url()
            )
        })?;
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
    }
    if let Some(expected) = expected.filter(|expected| *expected != written) {
        anyhow::bail!("Download from {shown} ended after {written} of {expected} bytes");
    }
    Ok(written)
}

async fn download_s3(
    url: &Url,
    file: &mut std::fs::File,
    config: &Configuration,
) -> anyhow::Result<u64> {
    use object_store::aws::{AmazonS3Builder, AmazonS3ConfigKey};
    use object_store::ObjectStore;

    let shown = display_url(url.as_str());
    let bucket = url
        .host_str()
        .filter(|bucket| !bucket.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{shown} names no bucket"))?;
    let key = object_store::path::Path::from_url_path(url.path())
        .map_err(|error| anyhow::anyhow!("{shown} has an unsupported object key: {error}"))?;
    if key.as_ref().is_empty() {
        anyhow::bail!("{shown} names no object");
    }
    // `with_client_options` replaces the client settings that `from_env`
    // read, such as `AWS_ALLOW_HTTP`, so they are read here again.
    let mut options = object_store::ClientOptions::new();
    for (key, value) in std::env::vars() {
        if !key.starts_with("AWS_") {
            continue;
        }
        if let Ok(AmazonS3ConfigKey::Client(key)) = key.to_ascii_lowercase().parse() {
            options = options.with_config(key, value);
        }
    }
    let store = AmazonS3Builder::from_env()
        .with_bucket_name(bucket)
        // No limit for a whole request, which would cut off large objects; a
        // stalled transfer fails in `next_chunk` instead.
        .with_client_options(
            options
                .with_timeout_disabled()
                .with_connect_timeout(config.timeouts.connect),
        )
        .with_retry(object_store::RetryConfig {
            max_retries: 3,
            retry_timeout: std::time::Duration::MAX,
            ..Default::default()
        })
        .build()
        .map_err(|error| anyhow::anyhow!("Can't configure S3 access: {error}"))?;
    let result = store.get(&key).await.map_err(|error| match error {
        object_store::Error::NotFound { .. } => anyhow::anyhow!("{shown} was not found"),
        error => anyhow::anyhow!("Request for {shown} failed: {error}"),
    })?;
    let expected = result.meta.size;
    let mut stream = result.into_stream();
    let mut written = 0u64;
    while let Some(chunk) =
        next_chunk(&mut stream, config.timeouts.request, &shown, written).await?
    {
        let chunk = chunk.map_err(|error| {
            anyhow::anyhow!("Download of {shown} was interrupted after {written} bytes: {error}")
        })?;
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
    }
    if written != expected {
        anyhow::bail!("Download of {shown} ended after {written} of {expected} bytes");
    }
    Ok(written)
}

/// Waits for the next chunk of a download for at most `idle`, so that a server
/// that stops sending can't stall the command. The whole download has no
/// time limit.
async fn next_chunk<S: futures_util::Stream + Unpin>(
    stream: &mut S,
    idle: std::time::Duration,
    shown: &str,
    written: u64,
) -> anyhow::Result<Option<S::Item>> {
    tokio::time::timeout(idle, stream.next())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Download from {shown} stalled: no data for {} seconds after {written} bytes",
                idle.as_secs_f64()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn complete_task() -> Value {
        json!({
            "id": "t1",
            "state": "COMPLETE",
            "outputs": [
                {"url": "s3://bucket/results/out.txt", "path": "/outputs/out.txt", "type": "FILE"},
                {"url": "https://store.example.org/dir", "path": "/outputs/dir", "type": "DIRECTORY"},
                {"url": "s3://bucket/glob", "path": "/outputs/*.bam", "path_prefix": "/outputs/"}
            ],
            "logs": [{
                "logs": [
                    {"exit_code": 0, "stdout": "hello\n", "stderr": ""},
                    {"exit_code": 1, "stdout": "two"}
                ],
                "system_logs": ["pulled image", "started"],
                "outputs": [
                    {"url": "s3://bucket/results/out.txt", "path": "/outputs/out.txt", "size_bytes": "6"},
                    {"url": "https://store.example.org/a/b%20c.bin?X-Amz-Signature=secret", "path": "", "size_bytes": 3}
                ]
            }]
        })
    }

    #[test]
    fn complete_task_lists_logs_and_outputs() {
        let catalog = Catalog::from_task(&complete_task());
        let selectors: Vec<_> = catalog.items.iter().map(|i| i.selector.as_str()).collect();
        assert_eq!(
            selectors,
            [
                "output:0",
                "output:1",
                "output:2",
                "stdout:0:0",
                "stderr:0:0",
                "stdout:0:1",
                "system:0",
                "file:0:0",
                "file:0:1"
            ]
        );
        assert_eq!(catalog.notes, ["stderr:0:1 is not in the task response."]);
        let system = catalog.find("system:0").unwrap();
        assert_eq!(
            system.content,
            Content::Inline("pulled image\nstarted\n".into())
        );
        let Content::File { size, .. } = &catalog.find("file:0:1").unwrap().content else {
            panic!("file expected");
        };
        assert_eq!(*size, Some(3));
        let table = catalog.table();
        assert!(table.contains("file:0:0       6          output file: s3://bucket/results/out.txt (path /outputs/out.txt)"), "{table}");
        assert!(!table.contains("secret"), "{table}");
    }

    #[test]
    fn server_values_in_the_table_are_escaped() {
        let task = json!({"state": "COMPLETE", "logs": [{
            "logs": [{"exit_code": "\u{9b}2J\u{202e}", "stdout": ""}],
        }]});
        let table = Catalog::from_task(&task).table();
        assert!(table.contains(r#"exit code "\u{9b}2J\u{202e}""#), "{table}");
        assert!(!table.contains(['\u{9b}', '\u{202e}']), "{table}");
    }

    #[test]
    fn incomplete_task_is_reported() {
        let catalog = Catalog::from_task(&json!({"id": "t1", "state": "RUNNING"}));
        assert!(catalog.items.is_empty());
        assert_eq!(
            catalog.notes,
            [
                "The task is in state RUNNING; its logs and outputs may be incomplete.",
                "The task has no logs yet."
            ]
        );
        let catalog = Catalog::from_task(&json!({"logs": [{}]}));
        assert!(catalog.notes.contains(
            &"The task has no state; its logs and outputs may be incomplete.".to_string()
        ));
        assert!(catalog
            .notes
            .contains(&"Attempt 0 has no executor logs.".to_string()));
        assert!(catalog
            .notes
            .contains(&"Attempt 0 has no system logs.".to_string()));
        let error = catalog.find("stdout:0:0").unwrap_err();
        assert!(error.0.contains("has no stdout:0:0"), "{error}");
    }

    #[test]
    fn malformed_entries_become_notes() {
        let task = json!({"state": "COMPLETE", "outputs": [{"path": "/x"}], "logs": [{
            "logs": [{"exit_code": 0, "stdout": "", "stderr": ""}],
            "system_logs": [],
            "outputs": [{"path": "/y"}, {"url": "s3://b/k", "size_bytes": "many"}]
        }]});
        let catalog = Catalog::from_task(&task);
        assert!(catalog.notes.contains(&"output:0 has no URL.".to_string()));
        assert!(catalog.notes.contains(&"file:0:0 has no URL.".to_string()));
        assert!(catalog
            .notes
            .contains(&"file:0:1 has no valid size_bytes.".to_string()));
        assert!(Catalog::from_task(&json!({"outputs": 1}))
            .notes
            .contains(&"The task's outputs are not a list.".to_string()));
    }

    #[test]
    fn selectors_are_validated_and_normalized() {
        assert_eq!(parse_selector("stdout:00:1").unwrap(), "stdout:0:1");
        assert_eq!(parse_selector("output:3").unwrap(), "output:3");
        for invalid in [
            "",
            "stdout",
            "stdout:0",
            "stdout:0:0:0",
            "output:-1",
            "output:+1",
            "file:a:0",
            "logs:0",
            "system:",
            "STDOUT:0:0",
            "output:99999999999999999999999",
        ] {
            let error = parse_selector(invalid).unwrap_err();
            assert!(
                error.0.starts_with("Invalid selector"),
                "{invalid}: {error}"
            );
        }
        assert!(parse_selector("x\u{1b}[2J")
            .unwrap_err()
            .0
            .contains(r"\u{1b}"));
    }

    #[test]
    fn urls_are_shown_without_secrets() {
        assert_eq!(
            display_url("https://user:pw@h.example.org/a/b?sig=1#f"),
            "https://h.example.org/a/b?..."
        );
        assert_eq!(display_url("s3://bucket/key"), "s3://bucket/key");
        assert_eq!(display_url("not a url"), "(malformed URL)");
    }

    #[test]
    fn logs_are_escaped_per_line() {
        assert_eq!(printable_lines("a\nb\u{1b}[2J\r\n"), "a\nb\\u{1b}[2J\\r\n");
        assert_eq!(printable_lines("no newline"), "no newline\n");
        assert_eq!(printable_lines(""), "");
    }

    #[test]
    fn user_names_must_be_plain() {
        assert_eq!(check_user_name("out.txt").unwrap(), "out.txt");
        assert!(check_user_name("/etc/passwd")
            .unwrap_err()
            .0
            .contains("absolute"));
        assert!(check_user_name("../out").unwrap_err().0.contains("'..'"));
        assert!(check_user_name("..").unwrap_err().0.contains("'..'"));
        for invalid in ["a/b", "a\\b", ".", "", "a\nb"] {
            assert!(
                check_user_name(invalid).unwrap_err().0.contains("plain"),
                "{invalid:?}"
            );
        }
    }

    fn file_item(url: &str, path: Option<&str>) -> Item {
        Item {
            selector: "file:0:0".into(),
            description: String::new(),
            content: Content::File {
                url: url.into(),
                path: path.map(Into::into),
                size: None,
                is_directory: false,
            },
        }
    }

    #[test]
    fn default_names_come_from_the_path_or_url() {
        let name = |url, path| default_name(&file_item(url, path));
        assert_eq!(
            name("s3://b/k", Some("/outputs/out.txt")).unwrap(),
            "out.txt"
        );
        assert_eq!(name("s3://b/k", Some("/outputs/dir/")).unwrap(), "dir");
        assert_eq!(
            name("https://h/a/b%20c.bin?x=1", Some("")).unwrap(),
            "b c.bin"
        );
        assert_eq!(name("https://h/a/b.bin/", None).unwrap(), "b.bin");
        assert!(name("https://h/", None).unwrap_err().0.contains("--name"));
        assert!(name("https://h/a/..", None)
            .unwrap_err()
            .0
            .contains("--name"));
        assert!(name("https://h/a/%2E%2E", None)
            .unwrap_err()
            .0
            .contains("--name"));
        assert!(name("https://h/a/x%2Fy", None)
            .unwrap_err()
            .0
            .contains("plain"));
        assert!(name("s3://b/k", Some("/outputs/.."))
            .unwrap_err()
            .0
            .contains("plain"));
        assert!(name("s3://b/k", Some("/o/a\u{202e}b"))
            .unwrap_err()
            .0
            .contains(r"\u{202e}"));
        let inline = Item {
            selector: "stdout:0:1".into(),
            description: String::new(),
            content: Content::Inline(String::new()),
        };
        assert_eq!(default_name(&inline).unwrap(), "stdout-0-1.txt");
    }

    #[test]
    fn plans_are_checked_before_any_download() {
        let catalog = Catalog::from_task(&complete_task());
        let dir = tempfile::TempDir::new().unwrap();
        let selectors = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        let plan_error = |list: &[&str], name: Option<&str>| {
            plan(&catalog, &selectors(list), name, dir.path(), false)
                .err()
                .unwrap()
                .0
        };
        assert!(plan_error(&["output:1"], None).contains("directory or a pattern"));
        assert!(plan_error(&["output:2"], None).contains("directory or a pattern"));
        assert!(plan_error(&["stdout:0:0", "stdout:00:0"], None).contains("more than once"));
        assert!(plan_error(&["output:0", "file:0:0"], None).contains("saved as 'out.txt'"));
        assert!(plan_error(&["output:0", "file:0:0"], Some("x")).contains("exactly one"));
        std::fs::write(dir.path().join("stdout-0-0.txt"), "old").unwrap();
        assert!(plan_error(&["stdout:0:0"], None).contains("--force"));
        assert!(plan(
            &catalog,
            &selectors(&["stdout:0:0"]),
            None,
            dir.path(),
            true
        )
        .is_ok());
        let planned = plan(
            &catalog,
            &selectors(&["file:0:0"]),
            Some("x.txt"),
            dir.path(),
            false,
        )
        .unwrap();
        assert_eq!(planned[0].name, "x.txt");
    }

    #[test]
    fn inline_logs_are_shown_but_files_are_not() {
        let catalog = Catalog::from_task(&complete_task());
        assert_eq!(show(&catalog, "stdout:0:0").unwrap().stdout, "hello\n");
        assert!(show(&catalog, "file:0:0")
            .err()
            .unwrap()
            .0
            .contains("tes save"));
    }
}
