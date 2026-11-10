use clap::{Arg, Command};
use ga4gh_sdk::{clients::ServiceType, utils::configuration::Configuration};
use protes_companion::{
    configured_transport, create_plan, explain, fetch, prepare_rerun, tes_client, Choices,
};
use serde_json::{json, Value};
fn read(path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("document exceeds 8 MiB".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let original = |name: &'static str| {
        Command::new(name)
            .arg(Arg::new("original-request").required(true))
            .arg(Arg::new("choices").long("choices"))
    };
    let m = Command::new("protes-companion")
        .subcommand_required(true)
        .subcommand(Command::new("explain").arg(Arg::new("record").required(true)))
        .subcommand(Command::new("fetch").arg(Arg::new("task-id").required(true)))
        .subcommand(original("prepare"))
        .subcommand(original("create"))
        .get_matches();
    let (name, args) = m.subcommand().unwrap();
    let value = match name {
        "explain" => serde_json::to_value(explain(
            read(args.get_one::<String>("record").unwrap())?,
            false,
        )?)?,
        "fetch" => {
            let config = Configuration::from_file(ServiceType::TES)?;
            serde_json::to_value(
                fetch(
                    &configured_transport(&config)?,
                    args.get_one::<String>("task-id").unwrap(),
                )
                .await?,
            )?
        }
        "prepare" | "create" => {
            let choices = args
                .get_one::<String>("choices")
                .map(|p| read(p).and_then(|v| Ok(serde_json::from_value::<Choices>(v)?)))
                .transpose()?
                .unwrap_or_default();
            let plan = prepare_rerun(
                &read(args.get_one::<String>("original-request").unwrap())?,
                choices,
            )?;
            if name == "prepare" {
                serde_json::to_value(plan)?
            } else {
                let config = Configuration::from_file(ServiceType::TES)?;
                let tes = tes_client(&config).await?;
                let created = create_plan(&tes, &plan).await?;
                json!({"task_id":created.id,"plan":plan})
            }
        }
        _ => unreachable!(),
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
