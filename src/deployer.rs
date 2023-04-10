use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use eyre::{ensure, eyre, Result, WrapErr};
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::TypeTag;
use shared_crypto::intent::Intent;
use sui_framework::build_move_package;
use sui_keys::keystore::{AccountKeystore, FileBasedKeystore, Keystore};
use sui_sdk::rpc_types::{
    Balance, Coin, OwnedObjectRef, SuiObjectDataOptions, SuiObjectResponse,
    SuiTransactionBlockEffectsV1, SuiTransactionBlockResponse, SuiTransactionBlockResponseOptions,
};
use sui_sdk::{SuiClient, SuiClientBuilder};
use sui_types::base_types::{ObjectID, ObjectRef, SuiAddress};
use sui_types::crypto::{EmptySignInfo, Signature};
use sui_types::message_envelope::VerifiedEnvelope;
use sui_types::messages::{CallArg, ObjectArg, SenderSignedData, Transaction, TransactionData};
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use tracing::{info, instrument};

use crate::config::AppConfig;
use crate::constants::{
    MERGE_SUI_GAS_BUDGET, PUBLISH_PACKAGE_GAS_BUDGET, SETUP_PACKAGE_GAS_BUDGET,
};
use crate::publish_result::PublishResult;
use crate::transaction::{AssertSuccess, TryIntoEffects};

pub struct Deployer {
    pub keystore: Keystore,
    pub client: Arc<SuiClient>,
    pub active_address: SuiAddress,
    pub config: AppConfig,
}

impl Deployer {
    #[instrument(name = "Creating Deployer", skip_all)]
    pub async fn build(config: AppConfig) -> Result<Self> {
        let keystore_path = config
            .sui
            .keystore_path()
            .wrap_err("Failed to get keystore path")?;

        let keystore: Keystore = FileBasedKeystore::new(&keystore_path)
            .map_err(|e| eyre!(e))?
            .into();

        let sui_client = SuiClientBuilder::default()
            .build(config.sui.node_url.clone())
            .await
            .wrap_err("Failed to connect to Sui Node")?;

        let active_address = *keystore.addresses().last().unwrap();
        info!("Active address is {active_address}");

        Ok(Self {
            keystore,
            client: Arc::new(sui_client),
            active_address,
            config: config.clone(),
        })
    }

    #[instrument(name = "Looking for coin for gas budget", skip(self))]
    pub async fn find_gas_coin_to_pay_gas_budget(&self, amount: u64) -> Result<(Coin, Vec<Coin>)> {
        let mut gas_coins = self
            .get_sui_coins()
            .await
            .wrap_err("Failed to get sui coins")?;

        gas_coins.sort_unstable_by_key(|c| c.balance);
        let target_idx = gas_coins
            .iter()
            .position(|coin| coin.balance >= amount)
            .ok_or_else(|| eyre!("There isn't coin with balance higher than provided amount"))?;

        let target = gas_coins.swap_remove(target_idx);
        Ok((target, gas_coins))
    }

    #[instrument(name = "Merging all gas", skip(self))]
    pub async fn merge_all_gas(&mut self) -> Result<(u64, ObjectID)> {
        let gas_budget = MERGE_SUI_GAS_BUDGET * 2;
        let (gas_payer, coins_to_merge) = self
            .find_gas_coin_to_pay_gas_budget(gas_budget)
            .await
            .wrap_err("Failed to find suitable gas coin to pay for merging all gas coins")?;

        let (target, resource) = coins_to_merge
            .split_first()
            .ok_or_else(|| eyre!("You don't have enough gas coins for merging. You must have one suitable for paying gas, one target coin and resource coins"))?;

        ensure!(
            !resource.is_empty(),
            "You must to have at least one resource coin"
        );

        let mut ret = target.balance;
        let mut builder = ProgrammableTransactionBuilder::default();
        let sui_framework_package = ObjectID::from_hex_literal(
            "0x0000000000000000000000000000000000000000000000000000000000000002",
        )
        .wrap_err("Failed to create object id for sui-framework packages")?;
        let pay_module = Identifier::from_str("pay")
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to create identifier for sui-framework module")?;
        let join_function = Identifier::from_str("join")
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to create identifier for sui-framework function")?;
        let sui_coin_type_arg = TypeTag::from_str(&gas_payer.coin_type)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to create type tag for SUI")?;

        for coin in resource {
            let call_args = vec![
                CallArg::Object(ObjectArg::ImmOrOwnedObject(target.object_ref())),
                CallArg::Object(ObjectArg::ImmOrOwnedObject(coin.object_ref())),
            ];

            builder
                .move_call(
                    sui_framework_package,
                    pay_module.clone(),
                    join_function.clone(),
                    vec![sui_coin_type_arg.clone()],
                    call_args,
                )
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to add move call in programmable tx builder")?;

            ret += coin.balance;
        }

        let gas_price = self
            .client
            .read_api()
            .get_reference_gas_price()
            .await
            .wrap_err("Failed to get gas price")?;
        let pt = builder.finish();

        let tx_data = TransactionData::new_programmable(
            self.active_address,
            vec![gas_payer.object_ref()],
            pt,
            gas_budget,
            gas_price,
        );

        let signature = self
            .sign(&tx_data)
            .wrap_err("Failed to sign data for merge tx")?;

        let tx = verify_tx_data(tx_data, signature)
            .wrap_err("Failed to verify tx data for merging gas")?;

        self.execute_tx(tx)
            .await
            .wrap_err("Failed to execute tx with gas merging")?
            .try_into_effects()?
            .assert_success()
            .wrap_err("Failed to merge gas")?;

        info!("Merged gas successfully, total amount is {ret}");
        Ok((ret, target.coin_object_id))
    }

    #[instrument(name = "Getting SUI objects", skip(self))]
    async fn get_sui_coins(&self) -> Result<Vec<Coin>> {
        let ret = self
            .client
            .coin_read_api()
            .get_coins(self.active_address, None, None, None)
            .await
            .wrap_err("Failed to fetch SUI objects")?
            .data;

        Ok(ret)
    }

    #[instrument(name = "Getting SUI balance", skip(self))]
    pub async fn sui_balance(&self, address: SuiAddress) -> Result<Balance> {
        self.client
            .coin_read_api()
            .get_balance(address, None)
            .await
            .wrap_err_with(|| format!("Failed to get SUI balance for address: {address}"))
    }

    #[instrument(name = "Publishing package", skip(self))]
    pub async fn publish_package(
        &mut self,
        package_path: &Path,
    ) -> Result<SuiTransactionBlockResponse> {
        let (gas_payer, _) = self
            .find_gas_coin_to_pay_gas_budget(PUBLISH_PACKAGE_GAS_BUDGET)
            .await
            .wrap_err("Failed to update gas for publishing package")?;

        let (published_dependencies, compiled_modules) = build_and_compile_package(package_path)?;

        let tx_data = self
            .client
            .transaction_builder()
            .publish(
                self.active_address,
                compiled_modules,
                published_dependencies,
                Some(gas_payer.coin_object_id),
                PUBLISH_PACKAGE_GAS_BUDGET,
            )
            .await
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to build transaction for publishing package")?;

        let signature = self
            .sign(&tx_data)
            .wrap_err("Failed to sign data for publish tx")?;

        let tx = verify_tx_data(tx_data, signature)
            .wrap_err("Failed to verify tx data for publishing package")?;

        let ret = self
            .execute_tx(tx)
            .await
            .wrap_err("Failed to execute tx with package publishing")?;

        Ok(ret)
    }

    #[instrument(name = "Executing transaction", skip_all)]
    async fn execute_tx(
        &self,
        tx: VerifiedEnvelope<SenderSignedData, EmptySignInfo>,
    ) -> Result<SuiTransactionBlockResponse> {
        self.client
            .quorum_driver()
            .execute_transaction_block(
                tx,
                SuiTransactionBlockResponseOptions::new().with_effects(),
                Some(sui_types::messages::ExecuteTransactionRequestType::WaitForLocalExecution),
            )
            .await
            .wrap_err("Failed to execute tx")
    }

    #[instrument(name = "Processing publish effects", skip_all)]
    pub async fn process_published_objects(
        &self,
        SuiTransactionBlockEffectsV1 { created, .. }: SuiTransactionBlockEffectsV1,
    ) -> Result<Vec<SuiObjectResponse>> {
        let mut tasks = Vec::new();
        let shared_client = Arc::clone(&self.client);

        for OwnedObjectRef { reference, .. } in created {
            let shared_client = Arc::clone(&shared_client);
            let task = async move {
                shared_client
                    .read_api()
                    .get_object_with_options(
                        reference.object_id,
                        SuiObjectDataOptions::new().with_type(),
                    )
                    .await
                    .wrap_err_with(|| {
                        format!("Failed to get object with id {}", reference.object_id)
                    })
            };

            tasks.push(tokio::spawn(task));
        }
        let mut ret = Vec::new();
        for task in tasks {
            let object = task.await.wrap_err("Failed to complete task")??;
            ret.push(object);
        }

        Ok(ret)
    }

    #[instrument(name = "Setting up package", skip_all)]
    pub async fn setup_package(&mut self, publish_result: PublishResult) -> Result<()> {
        let (gas_payer, _) = self
            .find_gas_coin_to_pay_gas_budget(SETUP_PACKAGE_GAS_BUDGET)
            .await
            .wrap_err("Failed to find gas coin to setup package")?;

        let mut builder = ProgrammableTransactionBuilder::default();

        let lemons_module = Identifier::from_str("lemons")
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to create identifier for lemons module")?;
        let debug_setup_function = Identifier::from_str("debug_setup")
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to create identifier debug_setup function")?;

        let [lemon_cap_ref, lemon_mint_config_ref] = self
            .get_objects_references::<2>(vec![
                publish_result.lemon_cap,
                publish_result.lemon_mint_config,
            ])
            .await
            .wrap_err("Failed to get lemon_cap and lemon mint config references")?;

        let lemons_type_tag =
            TypeTag::from_str(&format!("{}::lemons::Lemons", publish_result.package))
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to create type tag for Lemons")?;

        let type_args = vec![lemons_type_tag.clone(), lemons_type_tag];
        let call_args = vec![
            CallArg::Object(ObjectArg::ImmOrOwnedObject(lemon_cap_ref)),
            CallArg::Object(ObjectArg::SharedObject {
                id: publish_result.lemon_mint_config,
                initial_shared_version: lemon_mint_config_ref.1,
                mutable: true,
            }),
        ];

        builder
            .move_call(
                publish_result.package,
                lemons_module.clone(),
                debug_setup_function.clone(),
                type_args,
                call_args,
            )
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to add move call in programmable tx builder")?;

        let gas_price = self
            .client
            .read_api()
            .get_reference_gas_price()
            .await
            .wrap_err("Failed to get gas price")?;
        let pt = builder.finish();

        let tx_data = TransactionData::new_programmable(
            self.active_address,
            vec![gas_payer.object_ref()],
            pt,
            SETUP_PACKAGE_GAS_BUDGET,
            gas_price,
        );

        // let tx_data = self
        //     .client
        //     .transaction_builder()
        // .move_call(
        //     self.active_address,
        //     publish_result.package,
        //     "lemons",
        //     "debug_setup",
        //     Vec::new(),
        //     call_args,
        //     Some(ObjectID::from_hex_literal(GAS_OBJECT_ID).unwrap()),
        //     SETUP_PACKAGE_GAS_BUDGET,
        // )
        // .
        // .move_call(
        //     self.active_address,
        //     publish_result.package,
        //     "lemons",
        //     "debug_setup",
        //     Vec::new(),
        //     call_args,
        //     self.gas_object_id,
        //     SETUP_PACKAGE_GAS_BUDGET,
        // )
        // .await
        // .map_err(|e| eyre!(e))
        // .wrap_err("Failed to build transaction to setup package")?;

        let signature = self
            .sign(&tx_data)
            .wrap_err("Failed to sign data to setup package")?;

        let tx = verify_tx_data(tx_data, signature)
            .wrap_err("Failed to verify tx data to setup package")?;

        self.execute_tx(tx)
            .await
            .wrap_err("Failed to execute tx with package setup")?;

        Ok(())
    }

    #[instrument(name = "Signing transaction data", skip_all)]
    fn sign(&self, data: &TransactionData) -> Result<Signature> {
        let signature = self
            .keystore
            .sign_secure(&self.active_address, data, Intent::sui_transaction())
            .wrap_err("Failed to sign tx data")?;

        Ok(signature)
    }

    #[instrument(name = "Getting objects references", skip(self))]
    pub async fn get_objects_references<const N: usize>(
        &self,
        object_ids: Vec<ObjectID>,
    ) -> Result<[ObjectRef; N]> {
        let mut tasks = Vec::new();
        let shared_client = Arc::clone(&self.client);

        for object_id in object_ids {
            let shared_client = Arc::clone(&shared_client);
            let task = async move {
                shared_client
                    .read_api()
                    .get_object_with_options(object_id, SuiObjectDataOptions::default())
                    .await
                    .wrap_err_with(|| format!("Failed to get object with id {}", object_id))
            };

            tasks.push(tokio::spawn(task));
        }
        let mut ret = Vec::new();
        for task in tasks {
            let object = task.await.wrap_err("Failed to complete task")??;
            ret.push(object.object_ref_if_exists().unwrap());
        }

        Ok(<[_; N]>::try_from(ret).unwrap())
    }
}

type CompiledModules = Vec<Vec<u8>>;
type PublishedDependencies = Vec<ObjectID>;

#[instrument(name = "Building and compiling package")]
fn build_and_compile_package(
    package_path: &Path,
) -> Result<(PublishedDependencies, CompiledModules)> {
    let package = build_move_package(package_path, Default::default())
        .wrap_err("Failed to build move package")?;
    let dependencies: Vec<_> = package
        .dependency_ids
        .published
        .clone()
        .into_values()
        .collect();

    let modules = package.get_package_bytes(true);

    Ok((dependencies, modules))
}

#[instrument(name = "Verifying transaction data", skip_all)]
fn verify_tx_data(
    tx_data: TransactionData,
    signature: Signature,
) -> Result<VerifiedEnvelope<SenderSignedData, EmptySignInfo>> {
    Transaction::from_data(tx_data, Intent::sui_transaction(), vec![signature])
        .verify()
        .wrap_err("Failed to verify tx")
}
