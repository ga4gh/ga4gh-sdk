//! The `downstream-cli` binary: the SDK's CLI plus `tes usage`.
//!
//! Like `ga4gh-cli`, it reads each service's section from
//! `~/.ga4gh-cli/config.json`; `tes usage` reads the `TES` section.

use downstream_sdk::cli;
use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let matches = cli::command().get_matches();
    let (mut stdout, mut stderr) = (std::io::stdout(), std::io::stderr());
    let exit_code = if cli::usage_task_id(&matches).is_some() {
        let config = Configuration::from_file(ServiceType::TES)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        cli::run_extension(&matches, &config, &mut stdout, &mut stderr).await?
    } else {
        // Built-in commands read their own service's section.
        ga4gh_cli::dispatch_to(&matches, &mut stdout, &mut stderr).await?
    };
    Ok(exit_code.unwrap_or(ExitCode::FAILURE))
}
