use crate::constants::CONFIG_PATH;
use eyre::{eyre, Result, WrapErr};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::instrument;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub sui: SuiConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SuiConfig {
    pub config_path: String,
    pub keystore_filename: String,
    pub node_url: String,
    pub move_package_path: String,
}

impl SuiConfig {
    pub fn keystore_path(&self) -> Result<PathBuf> {
        let ret = dirs::home_dir()
            .ok_or_else(|| eyre!("Failed to get home directory"))?
            .join(self.config_path.as_str())
            .join(self.keystore_filename.as_str());

        Ok(ret)
    }

    pub fn move_package_path(&self) -> Result<PathBuf> {
        let ret = std::env::current_dir()
            .wrap_err("Failed to get current directory")?
            .parent()
            .ok_or_else(|| eyre!("Failed to get parent directory"))?
            .join(self.move_package_path.as_str());

        Ok(ret)
    }
}

#[instrument(name = "Loading config")]
pub fn load_config() -> Result<AppConfig> {
    let config_path = std::env::current_dir()
        .wrap_err("Failed to determine the current directory")?
        .join(CONFIG_PATH);

    let ret = config::Config::builder()
        .add_source(config::File::from(config_path))
        .build()
        .wrap_err("Failed to build config")?;

    ret.try_deserialize()
        .wrap_err("Failed to deserialize config into struct `Config`")
}
