//! Manage native extension installations without executing their code.

use crate::{CliContext, CommandOutput};
use clap::{Arg, ArgMatches, Command};
use ga4gh_sdk::utils::extension_manager::ExtensionManager;

/// Builds the `extension` command family.
pub fn command() -> Command {
    let mut command = Command::new("extension")
        .about("Manage experimental native extensions")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("list").about("List installed extensions"))
        .subcommand(
            Command::new("add")
                .about("Install a library from a JSON manifest")
                .arg(Arg::new("file").required(true)),
        );
    for (name, description) in [
        ("remove", "Remove an installed extension"),
        ("enable", "Enable an extension for explicit loading"),
        ("disable", "Disable an installed extension"),
    ] {
        command = command.subcommand(
            Command::new(name)
                .about(description)
                .arg(Arg::new("name").required(true)),
        );
    }
    command
}

/// Runs an installation command using `extensions.json` beside this context's
/// configuration file. A service section or profile is not needed. Libraries
/// are not loaded, including when enabling an extension.
///
/// # Errors
///
/// Returns configuration, input, download and filesystem errors. Callers own
/// output streams and decide how to display the returned output or error.
pub async fn run(matches: &ArgMatches, context: &CliContext) -> anyhow::Result<CommandOutput> {
    let config = context.config_path()?;
    let registry = config
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .join("extensions.json");
    let mut manager = ExtensionManager::open(registry)?;
    let Some((command, sub)) = matches.subcommand() else {
        anyhow::bail!("An extension subcommand is required");
    };
    if command == "list" {
        let mut output = format!("{:<25} {}\n", "Extension Name", "Enabled");
        for extension in manager.get_extensions() {
            output.push_str(&format!("{:<25} {}\n", extension.name, extension.enabled));
        }
        return Ok(CommandOutput::success(output));
    }
    let argument = if command == "add" { "file" } else { "name" };
    let value = sub
        .try_get_one::<String>(argument)
        .ok()
        .flatten()
        .ok_or_else(|| anyhow::anyhow!("Missing extension {argument}"))?;
    match command {
        "add" => manager.add_extension(value).await?,
        "remove" => manager.remove_extension(value)?,
        "enable" => manager.enable_extension(value)?,
        "disable" => manager.disable_extension(value)?,
        _ => anyhow::bail!("Unknown extension subcommand"),
    }
    Ok(CommandOutput::success(String::new()))
}
