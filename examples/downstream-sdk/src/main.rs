//! The `downstream-cli` binary: the SDK's CLI plus `tes usage`.
//!
//! Like `ga4gh-cli`, it reads each service's section from
//! `~/.ga4gh-cli/config.json`; `tes usage` reads the `TES` section.

use downstream_sdk::cli;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let matches = cli::command().get_matches();
    cli::run(&matches, &mut std::io::stdout(), &mut std::io::stderr()).await
}
