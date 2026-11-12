use log::error;
use std::io::Write;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // Log messages can hold a service's response body, so they are escaped
    // like the rest of the text output.
    let mut logger = env_logger::Builder::from_default_env();
    // The HTTP libraries log full URLs at debug level, including the query
    // strings of pre-signed output URLs. They log at debug level only when
    // `RUST_LOG` names them.
    let filters = std::env::var("RUST_LOG").unwrap_or_default();
    if filters.is_empty() {
        // env_logger's default level applies only when no filter is set.
        logger.filter_level(log::LevelFilter::Error);
    }
    for module in ["reqwest", "hyper", "hyper_util"] {
        if !filters.contains(module) {
            logger.filter_module(module, log::LevelFilter::Info);
        }
    }
    logger
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {} {}] {}",
                buf.timestamp(),
                record.level(),
                record.target(),
                ga4gh_cli::printable(&record.args().to_string())
            )
        })
        .init();

    let matches = ga4gh_cli::command().get_matches();
    match ga4gh_cli::dispatch_to(&matches, &mut std::io::stdout(), &mut std::io::stderr()).await? {
        Some(exit_code) => Ok(exit_code),
        None => {
            error!("Error: Unrecognized command or option");
            Ok(ExitCode::FAILURE)
        }
    }
}
