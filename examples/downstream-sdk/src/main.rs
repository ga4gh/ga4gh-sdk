//! The `downstream-cli` binary: the SDK's CLI plus `tes usage` and `quota`.
//!
//! It reads each service's section from `~/.downstream-cli/config.json`, and
//! a token from `DOWNSTREAM_CLI_TOKEN` for sections without credentials; see
//! [`downstream_sdk::cli::context`].

use downstream_sdk::cli;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let matches = cli::command().get_matches();
    cli::run(&matches, &mut std::io::stdout(), &mut std::io::stderr()).await
}
