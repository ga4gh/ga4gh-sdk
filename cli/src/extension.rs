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
        (
            "remove",
            "Unregister an extension; retain artifact files and disk usage",
        ),
        ("enable", "Enable an extension for explicit loading"),
        ("disable", "Disable an installed extension"),
    ] {
        command = command.subcommand(
            Command::new(name)
                .about(description)
                .arg(Arg::new("name").required(true)),
        );
    }
    command.subcommand(
        Command::new("select")
            .about("Select an installed artifact without loading it")
            .arg(Arg::new("name").required(true))
            .arg(Arg::new("digest").required(true)),
    )
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
        let mut output = format!(
            "{:<25} {:<12} {:<8} {:<9} {:<30} {}\n",
            "Extension Name", "Version", "Enabled", "Selected", "Target", "Digest"
        );
        for extension in manager.get_extensions() {
            for artifact in &extension.artifacts {
                let identity = &artifact.manifest.artifact;
                let version = identity.version.to_string();
                output.push_str(&format!(
                    "{:<25} {:<12} {:<8} {:<9} {:<30} {}\n",
                    extension.name,
                    version,
                    extension.enabled,
                    extension.selected == identity.digest,
                    identity.target,
                    identity.digest
                ));
            }
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
        "select" => manager.select_extension(
            value,
            sub.get_one::<String>("digest").expect("required digest"),
        )?,
        _ => anyhow::bail!("Unknown extension subcommand"),
    }
    Ok(CommandOutput::success(if command == "remove" {
        "Registration removed; artifact files retained. Disk space is not reclaimed.\n".to_owned()
    } else {
        String::new()
    }))
}
