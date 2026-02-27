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

    // 1234 = 0x04D2, so bytes[30]=0x04, bytes[31]=0xD2
    const ID_1234: AccountId = AccountId::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 4, 210,
    ]);
    // 1111 = 0x0457, so bytes[30]=0x04, bytes[31]=0x57
    const ID_1111: AccountId = AccountId::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 4, 87,
    ]);
    // 2222 = 0x08AE, so bytes[30]=0x08, bytes[31]=0xAE
    const ID_2222: AccountId = AccountId::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 8, 174,
    ]);
    const ID_1: AccountId = AccountId::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 1,
    ]);
    // 9999 = 0x270F, so bytes[30]=0x27, bytes[31]=0x0F
    const ID_9999: AccountId = AccountId::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 39, 15,
    ]);

    #[test]
    fn test_increase_same_id() {
        let mut asset1 = FungibleAsset {
            asset_id: ID_1234,
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: ID_1234,
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
            asset_id: ID_1234,
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: ID_1234,
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
            asset_id: ID_1111,
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: ID_2222,
            amount: 50,
        };
        let result = asset1.increase(asset2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::ERR_INCOMPATIBLE_FA);
        assert_eq!(asset1.amount, 100);
    }

    #[test]
    fn test_decrease_different_id() {
        let mut asset1 = FungibleAsset {
            asset_id: ID_1111,
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: ID_2222,
            amount: 50,
        };
        let result = asset1.decrease(asset2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::ERR_INCOMPATIBLE_FA);
        assert_eq!(asset1.amount, 100);
    }

    #[test]
    fn test_increase_overflow() {
        let mut asset1 = FungibleAsset {
            asset_id: ID_1,
            amount: u128::MAX,
        };
        let asset2 = FungibleAsset {
            asset_id: ID_1,
            amount: 1,
        };
        let result = asset1.increase(asset2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::ERR_OVERFLOW);
        assert_eq!(asset1.amount, u128::MAX);
    }

    #[test]
    fn test_decrease_insufficient_balance() {
        let mut asset1 = FungibleAsset {
            asset_id: ID_9999,
            amount: 100,
        };
        let asset2 = FungibleAsset {
            asset_id: ID_9999,
            amount: 200,
        };
        let result = asset1.decrease(asset2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::ERR_INSUFFICIENT_BALANCE);
        assert_eq!(asset1.amount, 100);
    }
}
