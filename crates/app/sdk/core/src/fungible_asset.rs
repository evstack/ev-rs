use crate::{AccountId, SdkResult};
use borsh::{BorshDeserialize, BorshSerialize};

/// A simple fungible asset with `account_id` and `amount`.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, Eq, PartialEq)]
pub struct FungibleAsset {
    pub asset_id: AccountId,
    pub amount: u128,
}

impl FungibleAsset {
    /// In-place "increase" of this asset's amount by `other.amount`.
    ///
    /// Consumes `other`. Returns an error if:
    /// - The `account_id` is different,
    /// - An overflow occurs (i.e., `amount + other.amount > u128::MAX`).
    pub fn increase(&mut self, other: Self) -> SdkResult<()> {
        if self.asset_id != other.asset_id {
            return Err(crate::ERR_INCOMPATIBLE_FA);
        }

        let new_amount = self
            .amount
            .checked_add(other.amount)
            .ok_or(crate::ERR_OVERFLOW)?;

        self.amount = new_amount;
        Ok(())
    }

    /// In-place "decrease" of this asset's amount by `other.amount`.
    ///
    /// Consumes `other`. Returns an error if:
    /// - The `account_id` is different,
    /// - The balance would go negative (i.e., `self.amount < other.amount`).
    pub fn decrease(&mut self, other: Self) -> SdkResult<()> {
        if self.asset_id != other.asset_id {
            return Err(crate::ERR_INCOMPATIBLE_FA);
        }
        if self.amount < other.amount {
            return Err(crate::ERR_INSUFFICIENT_BALANCE);
        }

        self.amount -= other.amount;
        Ok(())
    }
}

//------------------------------------
// Tests
//------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_increase_same_id() {
        let mut asset1 = FungibleAsset {
            asset_id: AccountId::new(1234),
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: AccountId::new(1234),
            amount: 50,
        };
        let result = asset1.increase(asset2);
        assert!(result.is_ok());
        assert_eq!(asset1.amount, 150);
        // asset2 is now consumed; can't be used again
    }

    #[test]
    fn test_decrease_same_id() {
        let mut asset1 = FungibleAsset {
            asset_id: AccountId::new(1234),
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: AccountId::new(1234),
            amount: 80,
        };
        let result = asset1.decrease(asset2);
        assert!(result.is_ok());
        assert_eq!(asset1.amount, 20);
        // asset2 is now consumed; can't be used again
    }

    #[test]
    fn test_increase_different_id() {
        let mut asset1 = FungibleAsset {
            asset_id: AccountId::new(1111),
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: AccountId::new(2222),
            amount: 50,
        };
        let result = asset1.increase(asset2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::ERR_INCOMPATIBLE_FA);
    }

    #[test]
    fn test_decrease_insufficient_balance() {
        let mut asset1 = FungibleAsset {
            asset_id: AccountId::new(9999),
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: AccountId::new(9999),
            amount: 200,
        };
        let result = asset1.decrease(asset2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::ERR_INSUFFICIENT_BALANCE);
    }
}
