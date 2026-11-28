use log::error;
use std::io::Write;
use std::process::ExitCode;

/// The directives that env_logger applies for these `RUST_LOG` filters, by
/// name, with `None` for a global level.
///
/// This mirrors `parse_spec` of env_filter 2.0:
/// - Only the text before a `/` counts; with more than one `/`, nothing does.
/// - Each directive is trimmed, but the name before `=` is kept as it is, so
///   `reqwest =info` names `reqwest ` (with a space).
/// - Directives with an invalid level, or with more than one `=`, are skipped.
fn log_directives(filters: &str) -> Vec<Option<&str>> {
    let mut parts = filters.split('/');
    let directives = parts.next().unwrap_or_default();
    if parts.nth(1).is_some() {
        return Vec::new();
    }
    directives
        .split(',')
        .map(str::trim)
        .filter(|directive| !directive.is_empty())
        .filter_map(|directive| {
            let mut parts = directive.split('=');
            let name = parts.next().unwrap_or_default();
            match (parts.next().map(str::trim), parts.next()) {
                (None, _) if name.parse::<log::LevelFilter>().is_ok() => Some(None),
                (None, _) | (Some(""), None) => Some(Some(name)),
                (Some(level), None) if level.parse::<log::LevelFilter>().is_ok() => {
                    Some(Some(name))
                }
                _ => None,
            }
        })
        .collect()
}

/// Whether `RUST_LOG` holds a directive for exactly this target.
///
/// Directives for child modules such as `reqwest::connect` don't count:
/// env_logger applies the longest matching name, so they keep their level next
/// to a filter for the target itself.
fn names_log_target(filters: &str, module: &str) -> bool {
    log_directives(filters).contains(&Some(module))
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
    // Without directives, env_logger logs errors only, unless the CLI adds
    // directives of its own, which would silence all other targets.
    let filters = std::env::var("RUST_LOG").unwrap_or_default();
    if !log_directives(&filters).is_empty() {
        for module in ["reqwest", "hyper", "hyper_util"] {
            if !names_log_target(&filters, module) {
                logger.filter_module(module, log::LevelFilter::Info);
            }
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

    let context = ga4gh_cli::CliContext::default();
    let parsed = context
        .parse_from(context.command(), std::env::args_os())
        .unwrap_or_else(|error| error.exit());
    match parsed
        .dispatch_to(&mut std::io::stdout(), &mut std::io::stderr())
        .await?
    {
        Some(exit_code) => Ok(exit_code),
        None => {
            error!("Error: Unrecognized command or option");
            Ok(ExitCode::FAILURE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{log_directives, names_log_target};

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
    }

    #[test]
    fn reads_global_levels_and_skips_empty_directives() {
        assert_eq!(log_directives("debug, ,reqwest"), [None, Some("reqwest")]);
        assert_eq!(log_directives(" "), []);
        assert_eq!(log_directives("debug/a/b"), []);
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
        assert!(!names_log_target("debug/reqwest=info", "reqwest"));
    }

    #[test]
    fn names_the_target_but_not_its_children() {
        for filters in ["reqwest", "reqwest=debug", "reqwest/connect"] {
            assert!(names_log_target(filters, "reqwest"), "{filters:?}");
        }
        for filters in ["warn,reqwest::connect=debug", "reqwest::connect::http"] {
            assert!(!names_log_target(filters, "reqwest"), "{filters:?}");
        }
    }

    #[test]
    fn ignores_targets_with_a_shared_prefix_or_suffix() {
        for filters in [
            "trace,hyper=debug",
            "reqwest_middleware=debug",
            "reqwestx::connect=debug",
            "ga4gh_cli=debug,warn",
            "my::reqwest=debug",
        ] {
            assert!(!names_log_target(filters, "reqwest"), "{filters:?}");
        }
    }
}
