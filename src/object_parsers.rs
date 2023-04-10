use crate::publish_result::{PublishResult, PublishResultBuilder};
use eyre::{bail, eyre, WrapErr};
use move_core_types::language_storage::{StructTag, TypeTag};
use sui_types::base_types::{ObjectID, ObjectType};

pub fn process_objects(objects: Vec<(ObjectID, ObjectType)>) -> eyre::Result<PublishResult> {
    let mut ret_builder = PublishResultBuilder::default();
    let (mut packages, rest): (Vec<_>, Vec<_>) = objects
        .into_iter()
        .partition(|(_, r#type)| matches!(r#type, ObjectType::Package));

    let (package_id, _) = packages.pop().ok_or_else(|| {
        eyre!("Objects from publish transaction effects must contain one package object")
    })?;
    ret_builder.package(package_id);

    for (id, object_type) in rest {
        let StructTag {
            module,
            name,
            type_params,
            ..
        } = object_type.try_into().unwrap();

        match (module.as_str(), name.as_str()) {
            ("admin", "AdminCap") => admin_cap_parser(&mut ret_builder, id, type_params)?,
            ("mint_config", "MintConfig") => mint_config_parser(&mut ret_builder, id, type_params)?,
            ("registry", "Registry") => registry_parser(&mut ret_builder, id, type_params)?,
            ("randomness", "Randomness") => randomness_parser(&mut ret_builder, id, type_params)?,
            ("coin", "TreasuryCap") => coin_treasury_cap_parser(&mut ret_builder, id, type_params)?,
            ("lemon_pool", "LemonPool") => {
                ret_builder.lemons_pool(id);
            }
            ("lemons", "Treasury") => {
                ret_builder.lemon_treasury(id);
            }
            ("ljc", "JuiceTreasury") => {
                ret_builder.juice_treasury(id);
            }
            _ => continue,
        }
    }

    ret_builder.build().wrap_err("Failed to build.")
}

pub fn coin_treasury_cap_parser(
    builder: &mut PublishResultBuilder,
    id: ObjectID,
    type_params: Vec<TypeTag>,
) -> eyre::Result<()> {
    for param in type_params {
        let TypeTag::Struct(box StructTag { module, name, .. } ) = param else {
            bail!("CoinTreasury's type_params must contain only TypeTag::Struct");
       };

        match (module.as_str(), name.as_str()) {
            ("ljc", "LJC") => builder.coin_juice_treasury_cap(id),
            _ => continue,
        };
    }

    Ok(())
}

pub fn randomness_parser(
    builder: &mut PublishResultBuilder,
    id: ObjectID,
    type_params: Vec<TypeTag>,
) -> eyre::Result<()> {
    for param in type_params {
        let TypeTag::Struct(box StructTag { module, name, .. } ) = param else {
            bail!("Randomness's type_params must contain only TypeTag::Struct");
       };

        match (module.as_str(), name.as_str()) {
            ("lemons", "Lemons") => builder.lemon_randomness(id),
            _ => continue,
        };
    }

    Ok(())
}

pub fn registry_parser(
    builder: &mut PublishResultBuilder,
    id: ObjectID,
    type_params: Vec<TypeTag>,
) -> eyre::Result<()> {
    for param in type_params {
        let TypeTag::Struct(box StructTag { module, name, .. } ) = param else {
            bail!("Registry's type_params must contain only TypeTag::Struct");
       };

        match (module.as_str(), name.as_str()) {
            ("lemons", "Lemons") => builder.lemon_registry(id),
            _ => continue,
        };
    }

    Ok(())
}

pub fn mint_config_parser(
    builder: &mut PublishResultBuilder,
    id: ObjectID,
    type_params: Vec<TypeTag>,
) -> eyre::Result<()> {
    for param in type_params {
        let TypeTag::Struct(box StructTag { module, name, .. } ) = param else {
            bail!("MintConfig's type_params must contain only TypeTag::Struct");   
       };

        match (module.as_str(), name.as_str()) {
            ("lemons", "Lemons") => builder.lemon_mint_config(id),
            _ => continue,
        };
    }

    Ok(())
}

pub fn admin_cap_parser(
    builder: &mut PublishResultBuilder,
    id: ObjectID,
    type_params: Vec<TypeTag>,
) -> eyre::Result<()> {
    for param in type_params {
        let TypeTag::Struct(box StructTag { module, name, .. } ) = param else {
            bail!("AdminCap's type_params must contain only TypeTag::Struct");   
       };

        match (module.as_str(), name.as_str()) {
            ("ljc", "Juice") => builder.juice_cap(id),
            ("lemons", "Lemons") => builder.lemon_cap(id),
            _ => continue,
        };
    }

    Ok(())
}
