//! Test that missing storage attributes are rejected at compile time.

use evolve_macros::AccountState;

// Minimal mock types to make the test self-contained
mod mock_collections {
    pub struct Item<T>(std::marker::PhantomData<T>);
    impl<T> Item<T> {
        pub const fn new(_prefix: u8) -> Self {
            Self(std::marker::PhantomData)
        }
    }
}

use mock_collections::Item;

/// This should fail to compile because field_b is missing #[storage] attribute.
#[derive(AccountState)]
pub struct MissingStorageAttr {
    #[storage(0)]
    pub field_a: Item<u64>,
    pub field_b: Item<u64>,  // ERROR: missing #[storage] attribute
}

fn main() {}
