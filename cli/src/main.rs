use log::error;
use std::io::Write;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    env_logger::init();

    let matches = ga4gh_cli::command().get_matches();
    match ga4gh_cli::dispatch(&matches).await? {
        Some(output) => {
            std::io::stdout().write_all(output.stdout.as_bytes())?;
            std::io::stderr().write_all(output.stderr.as_bytes())?;
            Ok(output.exit_code)
        }
        None => {
            error!("Error: Unrecognized command or option");
            Ok(ExitCode::FAILURE)
        }
    }
}
