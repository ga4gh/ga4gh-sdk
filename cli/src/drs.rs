use crate::CommandOutput;
use clap::{arg, ArgMatches, Command};
use ga4gh_sdk::clients::drs::DRS;
use ga4gh_sdk::utils::configuration::Configuration;

/// Build the DRS commands for reading object metadata and access URLs.
pub fn command() -> Command {
    Command::new("drs")
        .about("Data Repository Service commands")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("get")
                .about("Get a DRS object's metadata")
                .arg(arg!(<OBJECT_ID> "DRS object ID")),
        )
        .subcommand(
            Command::new("access")
                .about("Get a DRS object's access URL")
                .arg(arg!(<OBJECT_ID> "DRS object ID"))
                .arg(arg!(<ACCESS_ID> "Access method ID")),
        )
}

/// Run a parsed DRS command using the supplied service configuration.
///
/// A service failure returns a failure `CommandOutput`; configuration and
/// argument errors are returned by the caller before this function runs.
pub async fn run(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    let (name, args) = sub
        .subcommand()
        .ok_or_else(|| anyhow::anyhow!("DRS subcommand is required"))?;
    let object_id = args
        .get_one::<String>("OBJECT_ID")
        .ok_or_else(|| anyhow::anyhow!("OBJECT_ID argument is required"))?;
    let access_id = if name == "access" {
        Some(
            args.get_one::<String>("ACCESS_ID")
                .ok_or_else(|| anyhow::anyhow!("ACCESS_ID argument is required"))?,
        )
    } else {
        None
    };

    let drs = match DRS::new(config).await {
        Ok(drs) => drs,
        Err(error) => return Ok(CommandOutput::failure(error)),
    };
    match (name, access_id) {
        ("get", None) => match drs.get_object_raw(object_id).await {
            Ok(object) => Ok(CommandOutput::json(object)),
            Err(error) => Ok(CommandOutput::failure(error)),
        },
        ("access", Some(access_id)) => match drs.get_access_url_raw(object_id, access_id).await {
            Ok(access_url) => Ok(CommandOutput::json(access_url)),
            Err(error) => Ok(CommandOutput::failure(error)),
        },
        _ => anyhow::bail!("Unrecognized DRS command: {name}"),
    }
}
