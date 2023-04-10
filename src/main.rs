#![feature(box_patterns)]

use crate::config::load_config;
use crate::transaction::TryIntoEffects;
use deployer::Deployer;
use eyre::{eyre, Result, WrapErr};

mod config;
mod constants;
mod deployer;
mod object_parsers;
mod publish_result;
mod telemetry;
mod transaction;

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = telemetry::get_subscriber("backend".into(), "info".into(), std::io::stdout);
    telemetry::init_subscriber(subscriber).wrap_err("Failed to init tracing subscriber")?;
    let config = load_config().wrap_err("Failed to load app config")?;
    let mut deployer = Deployer::build(config.clone())
        .await
        .wrap_err("Failed to build deployer")?;

    let move_package_path = config
        .sui
        .move_package_path()
        .wrap_err("Failed to get path to move package")?;
    let effects = deployer
        .publish_package(&move_package_path)
        .await
        .wrap_err("Failed to publish package")?
        .try_into_effects()
        .wrap_err("Failed to convert into effects")?;

    let created_objects: Vec<_> = deployer
        .process_published_objects(effects)
        .await?
        .into_iter()
        .filter_map(|response| response.data)
        .filter_map(|data| match data.type_ {
            None => None,
            Some(r#type) => Some((data.object_id, r#type)),
        })
        .collect();

    let result = object_parsers::process_objects(created_objects)
        .wrap_err("Failed to process objects from effects")?;

    dbg!(result);

    deployer
        .setup_package(result)
        .await
        .wrap_err("Failed to setup package")?;

    Ok(())
}
