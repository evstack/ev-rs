//! Test that duplicate storage prefixes are rejected at compile time.

use evolve_macros::AccountState;

// Minimal mock types to make the test self-contained
mod mock_collections {
    pub struct Item<T>(std::marker::PhantomData<T>);
    impl<T> Item<T> {
        pub const fn new(_prefix: u8) -> Self {
            Self(std::marker::PhantomData)
        }
    }

    pub struct Map<K, V>(std::marker::PhantomData<(K, V)>);
    impl<K, V> Map<K, V> {
        pub const fn new(_prefix: u8) -> Self {
            Self(std::marker::PhantomData)
        }
    }
}

use mock_collections::{Item, Map};

/// This should fail to compile because both fields use storage prefix 0.
#[derive(AccountState)]
pub struct DuplicatePrefix {
    #[storage(0)]
    pub field_a: Item<u64>,
    #[storage(0)]  // ERROR: duplicate storage prefix
    pub field_b: Item<u64>,
}

fn main() {}
