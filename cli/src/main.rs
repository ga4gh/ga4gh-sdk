use log::error;
use std::io::Write;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // Log messages can hold a service's response body, so they are escaped
    // like the rest of the text output.
    env_logger::Builder::from_default_env()
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
