//! Interpret revision-1 metadata without loading any provider.
use crate::{printable, CliContext};
use clap::{Arg, ArgAction, ArgMatches, Command};
use ga4gh_cli_protocol::{Invocation, CAPABILITY};
use ga4gh_sdk::utils::{
    extension::{ExtensionErrorKind, ResolvedExtension},
    extension_manager::ExtensionManager,
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    name: String,
    about: String,
    commands: Vec<Leaf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Leaf {
    name: String,
    arguments: Vec<Argument>,
    #[serde(default)]
    wes_submit: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Argument {
    name: String,
    #[serde(default)]
    long: bool,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    repeat: bool,
    requires: Option<String>,
    help: Option<String>,
}

fn valid(name: &str) -> bool {
    name.len() <= 64
        && name.starts_with(|ch: char| ch.is_ascii_lowercase())
        && name
            .bytes()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')
}

impl Group {
    fn command(&self, root: &Command) -> anyhow::Result<Command> {
        anyhow::ensure!(
            valid(&self.name) && (1..=64).contains(&self.commands.len()),
            "Invalid extension command group"
        );
        let mut group = Command::new(self.name.clone())
            .about(printable(&self.about))
            .subcommand_required(true);
        let mut names = HashSet::from(["help"]);
        for leaf in &self.commands {
            anyhow::ensure!(
                valid(&leaf.name) && names.insert(&leaf.name) && leaf.arguments.len() <= 64,
                "Invalid or duplicate extension command"
            );
            let mut command = Command::new(leaf.name.clone());
            let mut ids = HashSet::from(["help", "version"]);
            for arg in root.get_arguments().filter(|arg| arg.is_global_set()) {
                ids.insert(arg.get_id().as_str());
                ids.extend(arg.get_long());
                ids.extend(arg.get_all_aliases().into_iter().flatten());
            }
            for arg in &leaf.arguments {
                anyhow::ensure!(
                    valid(&arg.name)
                        && ids.insert(&arg.name)
                        && (arg.long || (arg.required && !arg.repeat)),
                    "Invalid or duplicate extension argument"
                );
                let mut item = Arg::new(arg.name.clone()).required(arg.required);
                if arg.long {
                    item = item.long(arg.name.clone());
                }
                if arg.repeat {
                    item = item.action(ArgAction::Append);
                }
                if let Some(other) = &arg.requires {
                    anyhow::ensure!(
                        other != &arg.name && leaf.arguments.iter().any(|arg| &arg.name == other),
                        "Invalid extension argument dependency"
                    );
                    item = item.requires(other.clone());
                }
                if let Some(help) = &arg.help {
                    item = item.help(printable(help));
                }
                command = command.arg(item);
            }
            group = group.subcommand(command);
        }
        Ok(group)
    }
}

pub(super) struct Binding {
    group: Group,
    artifact: Result<ResolvedExtension, ExtensionErrorKind>,
}

impl Binding {
    pub(super) fn invocation(
        &self,
        matches: &ArgMatches,
        profile: Option<String>,
    ) -> anyhow::Result<(&ResolvedExtension, Invocation, bool)> {
        let (_, group) = matches.subcommand().expect("parsed group");
        let (name, args) = group.subcommand().expect("required subcommand");
        let leaf = self
            .group
            .commands
            .iter()
            .find(|leaf| leaf.name == name)
            .expect("parsed command");
        let arguments = leaf
            .arguments
            .iter()
            .filter_map(|arg| {
                args.get_many::<String>(&arg.name)
                    .map(|values| (arg.name.clone(), values.cloned().collect()))
            })
            .collect();
        let resolved = self.artifact.as_ref().map_err(|kind| {
            anyhow::anyhow!("Extension provider for '{}': {kind:?}", self.group.name)
        })?;
        let invocation = Invocation {
            group: self.group.name.clone(),
            command: name.into(),
            profile,
            arguments,
        };
        Ok((resolved, invocation, leaf.wes_submit))
    }
}

pub(super) struct Catalog {
    pub(super) command: Command,
    pub(super) bindings: BTreeMap<String, Binding>,
    pub(super) manager: ExtensionManager,
    pub(super) hint: String,
}

impl Catalog {
    pub(super) fn discover(context: &CliContext, root: &Command) -> anyhow::Result<Self> {
        let manager = context.extension_manager()?;
        let mut command = root.clone();
        let mut bindings = BTreeMap::new();
        let mut hint = String::from(
            "For extension commands, install and enable a compatible provider with 'extension'.",
        );
        let mut names = HashSet::from(["help".to_owned()]);
        for builtin in root.get_subcommands() {
            names.insert(builtin.get_name().to_owned());
            names.extend(builtin.get_all_aliases().map(str::to_owned));
        }
        let records = manager.get_extensions();
        for record in records
            .iter()
            .filter(|record| record.enabled)
            .chain(records.iter().filter(|record| !record.enabled))
        {
            let manifest = &record
                .artifacts
                .iter()
                .find(|art| art.manifest.artifact.digest == record.selected)
                .expect("registry validates selection")
                .manifest;
            let Some(revisions) = manifest.capabilities.get(CAPABILITY) else {
                continue;
            };
            if !revisions.contains(&1) {
                if record.enabled {
                    hint.push_str(&format!(
                        " Provider '{}' has no compatible CLI revision.",
                        record.name
                    ));
                }
                continue;
            }
            let groups = manifest
                .capability_metadata
                .get(CAPABILITY)
                .and_then(|map| map.get(&1))
                .and_then(|metadata| Vec::<Group>::deserialize(metadata).ok());
            let Some(groups) = groups else {
                if !record.enabled {
                    continue;
                }
                anyhow::bail!("Invalid or missing CLI metadata for '{}'", record.name);
            };
            let artifact = manager
                .resolve(
                    &record.name,
                    None,
                    Some(&record.selected),
                    &[(CAPABILITY.into(), vec![1])].into(),
                )
                .map_err(|error| error.kind);
            for group in groups {
                if !record.enabled && names.contains(&group.name) {
                    continue;
                }
                let subtree = match group.command(root) {
                    Err(_) if !record.enabled => continue,
                    command => command?,
                };
                anyhow::ensure!(
                    names.insert(group.name.clone()),
                    "Extension command collision: '{}'",
                    printable(&group.name)
                );
                command = command.subcommand(subtree.hide(artifact.is_err()));
                bindings.insert(
                    group.name.clone(),
                    Binding {
                        group,
                        artifact: artifact.clone(),
                    },
                );
            }
        }
        Ok(Self {
            command,
            bindings,
            manager,
            hint,
        })
    }
}
