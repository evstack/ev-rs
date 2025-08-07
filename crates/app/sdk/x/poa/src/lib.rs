use evolve_core::account_impl;

#[account_impl(Poa)]
pub mod account {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_collections::unordered_map::UnorderedMap;
    use evolve_collections::vector::Vector;
    use evolve_collections::ERR_DATA_CORRUPTION;
    use evolve_cometbft_account_trait::consensus_account::{
        AbciValsetManagerAccount, ValidatorUpdate,
    };
    use evolve_core::{define_error, AccountId, Environment, SdkResult, ERR_UNAUTHORIZED};
    use evolve_events::EventsEmitter;
    use evolve_macros::{exec, init, query};
    use evolve_scheduler::begin_block_account_interface::BeginBlockAccountInterface;

    define_error!(ERR_NO_VALIDATORS, 0x1, "no validators left");
    define_error!(ERR_VALIDATOR_NOT_FOUND, 0x2, "validator not found");
    define_error!(
        ERR_VALIDATOR_ALREADY_EXISTS,
        0x3,
        "validator already exists"
    );

    #[derive(BorshSerialize, BorshDeserialize, Clone)]
    pub enum ValsetChange {
        Add(AccountId),
        Remove(AccountId),
    }

    impl ValsetChange {
        fn into_validator_update(
            self,
            account: &Poa,
            env: &dyn Environment,
        ) -> SdkResult<ValidatorUpdate> {
            let (account_id, power) = match self {
                ValsetChange::Add(id) => (id, 1),
                ValsetChange::Remove(id) => (id, 0),
            };

            let validator = account.validators.get(&account_id, env)?;
            let pub_key = evolve_cometbft_account_trait::consensus_account::Pubkey::Ed25519(
                validator
                    .pub_key
                    .try_into()
                    .map_err(|_| ERR_DATA_CORRUPTION)?,
            );

            // Return your ValidatorUpdate
            Ok(ValidatorUpdate { power, pub_key })
        }
    }

    #[derive(BorshSerialize, BorshDeserialize, Clone)]
    pub struct Validator {
        pub account: AccountId,
        pub pub_key: Vec<u8>,
    }

    #[derive(BorshSerialize, BorshDeserialize, Clone)]
    pub struct EventValidatorRemoved {
        pub account_id: AccountId,
    }

    pub struct Poa {
        scheduler_authority: Item<AccountId>,
        update_authority: Item<AccountId>,
        validators: UnorderedMap<AccountId, Validator>,
        valset_changes: Vector<ValsetChange>,
        events: EventsEmitter,
    }

    impl Default for Poa {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Poa {
        pub const fn new() -> Self {
            Self {
                scheduler_authority: Item::new(0),
                update_authority: Item::new(1),
                validators: UnorderedMap::new(2, 3, 4, 5),
                valset_changes: Vector::new(6, 7),
                events: EventsEmitter::new(),
            }
        }
        #[init]
        pub fn initialize(
            &self,
            update_authority: AccountId,
            scheduler_authority: AccountId,
            validators: Vec<Validator>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.update_authority.set(&update_authority, env)?;
            self.scheduler_authority.set(&scheduler_authority, env)?;
            for validator in validators {
                self.validators
                    .insert(&validator.account, &validator, env)?;
            }

            Ok(())
        }

        #[exec]
        pub fn remove_validator(
            &self,
            validator: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if self.update_authority.get(env)? != env.sender() {
                return Err(ERR_UNAUTHORIZED);
            }

            // ensure not empty
            if self.validators.len(env)? == 1 {
                return Err(ERR_NO_VALIDATORS);
            }

            // check it exists
            if self.validators.may_get(&validator, env)?.is_none() {
                return Err(ERR_VALIDATOR_NOT_FOUND);
            }

            self.validators.remove(&validator, env)?;
            self.valset_changes
                .push(&ValsetChange::Remove(validator), env)?;

            self.events.emit_event(
                "validator_removed",
                EventValidatorRemoved {
                    account_id: validator,
                },
                env,
            )?;
            Ok(())
        }

        #[exec]
        pub fn add_validator(
            &self,
            validator: Validator,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if self.update_authority.get(env)? != env.sender() {
                return Err(ERR_UNAUTHORIZED);
            }

            // check it does not already exist
            if self.validators.may_get(&validator.account, env)?.is_some() {
                return Err(ERR_VALIDATOR_ALREADY_EXISTS);
            }

            self.validators
                .insert(&validator.account, &validator, env)?;
            self.valset_changes
                .push(&ValsetChange::Add(validator.account), env)?;

            Ok(())
        }

        #[query]
        pub fn get_validator_set(&self, env: &dyn Environment) -> SdkResult<Vec<Validator>> {
            self.validators
                .iter(env)?
                .map(|v| v.map(|v| v.1))
                .collect::<SdkResult<Vec<_>>>()
        }

        #[query]
        pub fn get_valset_changes(&self, env: &dyn Environment) -> SdkResult<Vec<ValsetChange>> {
            self.valset_changes.iter(env)?.collect()
        }
    }

    impl BeginBlockAccountInterface for Poa {
        #[exec]
        fn do_begin_block(&self, env: &mut dyn Environment) -> SdkResult<()> {
            if self.scheduler_authority.get(env)? != env.sender() {
                return Err(ERR_UNAUTHORIZED);
            }

            // clear.
            loop {
                if self.valset_changes.pop(env)?.is_none() {
                    return Ok(());
                }
            }
        }
    }

    impl AbciValsetManagerAccount for Poa {
        #[query]
        fn valset_changes(&self, env: &dyn Environment) -> SdkResult<Vec<ValidatorUpdate>> {
            // get all valset changes
            let changes = self
                .valset_changes
                .iter(env)?
                .map(|change| {
                    let change = change?;
                    change.into_validator_update(self, env)
                })
                .collect::<SdkResult<Vec<_>>>()?;
            Ok(changes)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn success() {}
}
