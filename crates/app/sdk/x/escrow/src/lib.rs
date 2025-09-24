use evolve_core::account_impl;

#[account_impl(Escrow)]
pub mod escrow {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_collections::map::Map;
    use evolve_core::{
        define_error, ensure, AccountId, Environment, FungibleAsset, SdkResult, ERR_UNAUTHORIZED,
    };
    use evolve_fungible_asset::FungibleAssetInterfaceRef;
    use evolve_macros::{exec, init};
    use evolve_unique::unique::{UniqueId, UniqueRef};

    define_error!(ERR_LOCK_NOT_READY, 0x1, "lock not matured");

    #[derive(BorshDeserialize, BorshSerialize, Clone, Debug)]
    pub struct Lock {
        pub owner: AccountId,
        pub unlock_height: u64,
        pub funds: Vec<FungibleAsset>,
    }
    #[derive(evolve_core::AccountState)]
    pub struct Escrow {
        #[storage(0)]
        pub locks: Map<UniqueId, Lock>,
        #[storage(1)]
        pub unique: Item<UniqueRef>,
    }

    impl Escrow {
        #[init]
        pub fn initialize(
            &self,
            unique_account: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.unique.set(&UniqueRef::from(unique_account), env)?;
            Ok(())
        }

        #[exec(payable)]
        pub fn create_lock(
            &self,
            recipient: AccountId,
            unlock_height: u64,
            env: &mut dyn Environment,
        ) -> SdkResult<UniqueId> {
            let id = self.unique.get(env)?.next_unique_id(env)?;
            let funds = env.funds().to_vec();

            let lock = Lock {
                owner: recipient,
                unlock_height,
                funds,
            };

            self.locks.set(&id, &lock, env)?;

            Ok(id)
        }

        #[exec]
        pub fn withdraw_funds(
            &self,
            escrow_id: UniqueId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            let escrow = self.locks.get(&escrow_id, env)?;
            // ensure owner
            ensure!(escrow.owner == env.sender(), ERR_UNAUTHORIZED);

            // ensure height - use env.block() directly instead of cross-account query
            let block_height = env.block().height;
            ensure!(block_height >= escrow.unlock_height, ERR_LOCK_NOT_READY);

            // unlock funds
            for fund in escrow.funds {
                let token_ref = FungibleAssetInterfaceRef::new(fund.asset_id);
                token_ref.transfer(escrow.owner, fund.amount, env)?
            }

            // delete escrow
            self.locks.remove(&escrow_id, env)?;

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_lock() {}
}
