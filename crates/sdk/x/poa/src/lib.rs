pub mod account {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_collections::unordered_map::UnorderedMap;
    use evolve_core::{AccountId, Environment, SdkResult, ERR_UNAUTHORIZED};
    use evolve_scheduler::begin_block_account_interface::BeginBlockAccountInterface;
    use evolve_scheduler::end_block_account_interface::EndBlockAccountInterface;

    #[derive(BorshSerialize, BorshDeserialize, Clone)]
    pub struct Validator {
        pub account: AccountId,
        pub pub_key: Vec<u8>,
    }

    pub struct Poa {
        scheduler_authority: Item<AccountId>,
        update_authority: Item<AccountId>,
        validators: UnorderedMap<AccountId, Validator>,
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
            }
        }
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

        pub fn remove_validator(
            &self,
            _validator: AccountId,
            _env: &mut dyn Environment,
        ) -> SdkResult<()> {
            todo!("impl")
        }

        pub fn add_validator(
            &self,
            _validator: Validator,
            _env: &mut dyn Environment,
        ) -> SdkResult<()> {
            todo!()
        }

        pub fn get_validator_set(&self, _env: &mut dyn Environment) -> SdkResult<Vec<Validator>> {
            todo!()
        }
    }

    impl EndBlockAccountInterface for Poa {
        fn do_end_block(&self, env: &mut dyn Environment) -> SdkResult<()> {
            if self.scheduler_authority.get(env)? != env.sender() {
                return Err(ERR_UNAUTHORIZED);
            }

            todo!("process valset changes")
        }
    }

    impl BeginBlockAccountInterface for Poa {
        fn do_begin_block(&self, env: &mut dyn Environment) -> SdkResult<()> {
            if self.scheduler_authority.get(env)? != env.sender() {
                return Err(ERR_UNAUTHORIZED);
            }

            todo!("kill previoius valset changes ")
        }
    }
}
