use crate::account::NameServiceRef;
use evolve_core::{AccountId, Environment, SdkResult};
use evolve_macros::account_impl;

/// TODO: make this magic with daemon accounts
const GLOBAL_NAME_SERVICE_REF: NameServiceRef = NameServiceRef::new(AccountId::new(65535));

/// This is kind of magic right now, but NameService is a global account sitting at a very well known
/// address, contrary to others.
pub fn resolve_name(name: String, env: &dyn Environment) -> SdkResult<Option<AccountId>> {
    GLOBAL_NAME_SERVICE_REF.resolve_name(name, env)
}

/// Resolves the given name as an account ref.
pub fn resolve_as_ref<T: From<AccountId>>(
    name: String,
    env: &dyn Environment,
) -> SdkResult<Option<T>> {
    resolve_name(name.clone(), env).map(|option| option.map(From::from))
}

#[account_impl(NameService)]
pub mod account {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult};
    use evolve_macros::{exec, init, query};

    pub struct NameService {
        pub owner: Item<AccountId>,
        pub name_to_account: Map<String, AccountId>,
    }

    pub const ERR_UNAUTHORIZED: ErrorCode = 0;

    impl NameService {
        pub const fn new() -> Self {
            NameService {
                owner: Item::new(0),
                name_to_account: Map::new(1),
            }
        }
        #[init]
        pub fn initialize(
            &self,
            initial_names: Vec<(AccountId, String)>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.owner.set(&env.sender(), env)?;

            for (account, name) in initial_names {
                self.name_to_account.set(&name, &account, env)?;
            }

            Ok(())
        }

        #[exec]
        pub fn update_name(
            &self,
            name: String,
            account: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.updates_names(vec![(name, account)], env)
        }

        #[exec]
        pub fn updates_names(
            &self,
            names: Vec<(String, AccountId)>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if env.sender() != self.owner.get(env)?.unwrap() {
                return Err(ERR_UNAUTHORIZED);
            }

            for (name, account) in names {
                self.name_to_account.set(&name, &account, env)?;
            }

            Ok(())
        }

        #[query]
        pub fn resolve_name(
            &self,
            name: String,
            env: &dyn Environment,
        ) -> SdkResult<Option<AccountId>> {
            self.name_to_account.get(&name, env)
        }
    }
}
