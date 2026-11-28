//! The `downstream-cli` binary: the SDK's CLI plus `tes usage` and `quota`.
//!
//! It reads each service's section from `~/.downstream-cli/config.json`, and
//! a token from `DOWNSTREAM_CLI_TOKEN` for sections without credentials; see
//! [`downstream_sdk::cli::context`].

use downstream_sdk::cli;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let parsed = cli::context()
        .parse_from(cli::command(), std::env::args_os())
        .unwrap_or_else(|error| error.exit());
    if let Some(output) = parsed.dispatch_extension(None).await? {
        return Ok(output.write_to(&mut std::io::stdout(), &mut std::io::stderr())?);
    }
    cli::run(
        parsed.matches(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
    .await
}
