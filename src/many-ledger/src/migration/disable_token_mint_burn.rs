use crate::migration::MIGRATIONS;
use linkme::distributed_slice;
use many_error::ManyError;
use many_migration::InnerMigration;

#[distributed_slice(MIGRATIONS)]
pub static DISABLE_TOKEN_MINT_BURN_MIGRATION: InnerMigration<merk::Merk, ManyError> =
    InnerMigration::new_trigger(
        false,
        "Disable Token Mint/Burn Migration",
        "Disables token mint/burn for all",
    );
