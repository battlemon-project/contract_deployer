use eyre::{bail, Result};
use sui_sdk::rpc_types::{
    SuiExecutionStatus, SuiTransactionBlockEffects, SuiTransactionBlockEffectsV1,
    SuiTransactionBlockResponse,
};

pub trait TryIntoEffects: Sized {
    fn try_into_effects(self) -> Result<SuiTransactionBlockEffectsV1>;
}

pub trait AssertSuccess: Sized {
    fn assert_success(self) -> Result<SuiTransactionBlockEffectsV1>;
}

impl TryIntoEffects for SuiTransactionBlockResponse {
    fn try_into_effects(self) -> Result<SuiTransactionBlockEffectsV1> {
        match self.effects {
            None => bail!("Transaction doesn't have effects, enable it in `SuiTransactionBlockResponseOptions` "),
            Some(SuiTransactionBlockEffects::V1(effects)) => Ok(effects),
        }
    }
}

impl AssertSuccess for SuiTransactionBlockEffectsV1 {
    fn assert_success(self) -> Result<SuiTransactionBlockEffectsV1> {
        match self {
            SuiTransactionBlockEffectsV1 {
                status: SuiExecutionStatus::Success,
                ..
            } => Ok(self),
            SuiTransactionBlockEffectsV1 {
                status: SuiExecutionStatus::Failure { error },
                ..
            } => bail!("Transaction effects contains error: {error}"),
        }
    }
}
