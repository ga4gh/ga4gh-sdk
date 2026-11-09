use log::error;
use std::error::Error;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> Result<ExitCode, Box<dyn Error>> {
    env_logger::init();

    let matches = ga4gh_cli::command().get_matches();
    match ga4gh_cli::dispatch(&matches).await? {
        Some(code) => Ok(code),
        None => {
            error!("Error: Unrecognized command or option");
            Ok(ExitCode::FAILURE)
        }
    }
}
