use derive_builder::Builder;
use eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use sui_types::base_types::ObjectID;

#[derive(Debug, Clone, Copy, Builder, Serialize, Deserialize)]
pub struct PublishResult {
    pub package: ObjectID,
    pub lemons_pool: ObjectID,
    pub lemon_registry: ObjectID,
    pub lemon_randomness: ObjectID,
    pub lemon_mint_config: ObjectID,
    pub lemon_treasury: ObjectID,
    pub lemon_cap: ObjectID,
    pub juice_cap: ObjectID,
    pub juice_treasury: ObjectID,
    pub coin_juice_treasury_cap: ObjectID,
}

impl PublishResult {
    pub fn to_file(&self) -> Result<()> {
        let path = std::env::current_dir()
            .wrap_err("Failed to read current dir")?
            .join("export.json");

        let file = File::create(path).wrap_err("Failed to create file")?;
        serde_json::to_writer_pretty(file, self).wrap_err("Failed to serialize data into file")?;

        Ok(())
    }

    pub fn from_file(path: PathBuf) -> Result<Self> {
        let file = std::fs::File::open(path).wrap_err("Failed to open file with publish result")?;
        serde_json::from_reader(file).wrap_err("Failed to deserialize file into struct")
    }
}
