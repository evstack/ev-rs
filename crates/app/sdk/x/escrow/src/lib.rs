use evolve_macros::account_impl;

#[account_impl(Escrow)]
pub mod escrow {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_block_info::account::BlockInfoRef;
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
    pub struct Escrow {
        pub locks: Map<UniqueId, Lock>,

        pub unique: Item<UniqueRef>,
        pub block_info: Item<BlockInfoRef>,
    }

    impl Default for Escrow {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Escrow {
        pub fn new() -> Self {
            Self {
                locks: Map::new(0),
                unique: Item::new(1),
                block_info: Item::new(2),
            }
        }
        #[init]
        pub fn initialize(
            &self,
            unique_account: AccountId,
            block_info_account: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.unique.set(&UniqueRef::from(unique_account), env)?;
            self.block_info
                .set(&BlockInfoRef::from(block_info_account), env)?;
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

            // ensure height
            let block_height = self.block_info.get(env)?.get_height(env)?;
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
