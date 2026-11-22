use log::error;
use std::io::Write;
use std::process::ExitCode;

/// Whether `RUST_LOG` holds a directive for exactly this target.
///
/// The filters are read as env_logger reads them: the text before a `/` (with
/// more than one `/`, env_logger ignores all filters), split at `,`, each
/// directive trimmed, and the name before `=` taken as it is, so `reqwest =info`
/// names `reqwest ` (with a space), not `reqwest`. env_logger skips directives
/// with an invalid level, so they name no target either. Directives for child
/// modules such as `reqwest::connect` don't count: env_logger applies the
/// longest matching name, so they keep their level next to a filter for the
/// target itself.
fn names_log_target(filters: &str, module: &str) -> bool {
    let mut parts = filters.split('/');
    let directives = parts.next().unwrap_or_default();
    if parts.nth(1).is_some() {
        return false;
    }
    directives.split(',').map(str::trim).any(|directive| {
        let mut parts = directive.split('=');
        let name = parts.next().unwrap_or_default();
        let level_is_valid = match (parts.next().map(str::trim), parts.next()) {
            (None, _) | (Some(""), None) => true,
            (Some(level), None) => level.parse::<log::LevelFilter>().is_ok(),
            (Some(_), Some(_)) => false,
        };
        name == module && level_is_valid
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // Log messages can hold a service's response body, so they are escaped
    // like the rest of the text output.
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"));
    // The HTTP libraries log full URLs at debug level, including the query
    // strings of pre-signed output URLs. They log at debug level only when
    // `RUST_LOG` names them exactly.
    let filters = std::env::var("RUST_LOG").unwrap_or_default();
    for module in ["reqwest", "hyper", "hyper_util"] {
        if !names_log_target(&filters, module) {
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

#[cfg(test)]
mod tests {
    use super::names_log_target;

    #[test]
    fn names_exact_targets() {
        assert!(names_log_target("debug,reqwest=info", "reqwest"));
        assert!(names_log_target("reqwest", "reqwest"));
        assert!(names_log_target("reqwest=", "reqwest"));
        assert!(names_log_target(" reqwest=info ,debug", "reqwest"));
        assert!(names_log_target("reqwest= INFO", "reqwest"));
        assert!(names_log_target("debug,reqwest=trace/regex", "reqwest"));
    }

    #[test]
    fn ignores_names_with_spaces() {
        assert!(!names_log_target("debug,reqwest =info", "reqwest"));
        assert!(!names_log_target("debug, reqwest = info", "reqwest"));
    }

    #[test]
    fn ignores_directives_env_logger_rejects() {
        assert!(!names_log_target("debug,reqwest=verbose", "reqwest"));
        assert!(!names_log_target("debug,reqwest=info=debug", "reqwest"));
        assert!(!names_log_target("reqwest=info/a/b", "reqwest"));
        assert!(!names_log_target("debug/reqwest=info", "reqwest"));
    }

    #[test]
    fn ignores_other_targets() {
        assert!(!names_log_target("", "reqwest"));
        assert!(!names_log_target("debug", "reqwest"));
        assert!(!names_log_target("debug,notreqwest=trace", "reqwest"));
        assert!(!names_log_target("debug,reqwe=trace", "reqwest"));
        assert!(!names_log_target("debug,reqwest_x=trace", "reqwest"));
        assert!(!names_log_target("debug,hyper=info", "hyper_util"));
        assert!(!names_log_target("debug,reqwest::connect=trace", "reqwest"));
    }
}
