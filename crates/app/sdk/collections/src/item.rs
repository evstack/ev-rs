use crate::map::Map;
use crate::ERR_NOT_FOUND;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{Environment, EnvironmentQuery, SdkResult};

pub struct Item<V>(Map<(), V>);

impl<V> Item<V> {
    pub const fn new(prefix: u8) -> Item<V> {
        Item(Map::new(prefix))
    }
}

impl<V> Item<V>
where
    V: Encodable + Decodable,
{
    /// Returns [`Ok(V)`] if it exists, or an [`Err`].
    pub fn get(&self, backend: &mut dyn EnvironmentQuery) -> SdkResult<V> {
        self.may_get(backend)?.ok_or(ERR_NOT_FOUND)
    }
    /// Returns [`Some(V)`] if the item exists, otherwise [`None`].
    pub fn may_get(&self, backend: &mut dyn EnvironmentQuery) -> SdkResult<Option<V>> {
        self.0.may_get(&(), backend)
    }

    pub fn set(&self, value: &V, backend: &mut dyn Environment) -> SdkResult<()> {
        self.0.set(&(), value, backend)
    }

    pub fn update(
        &self,
        update_fn: impl FnOnce(Option<V>) -> SdkResult<V>,
        backend: &mut dyn Environment,
    ) -> SdkResult<V> {
        self.0.update(&(), update_fn, backend)
    }

    pub fn remove(&self, env: &mut dyn Environment) -> SdkResult<()> {
        self.0.remove(&(), env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mocks::MockEnvironment;
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::ErrorCode;

    // Test data structure that implements Encodable and Decodable via BorshSerialize and BorshDeserialize
    #[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Clone)]
    struct TestData {
        value: u32,
        name: String,
    }

    #[test]
    fn test_get_nonexistent() {
        // Test getting a value that doesn't exist
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::new(1, 2);

        let result = item.may_get(&mut env);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_set_and_get() {
        // Test setting and then getting a value
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::new(1, 2);

        let test_data = TestData {
            value: 123,
            name: "test".to_string(),
        };

        // Set the value
        let set_result = item.set(&test_data, &mut env);
        assert!(set_result.is_ok());

        // Get the value
        let get_result = item.may_get(&mut env);
        assert!(get_result.is_ok());

        let retrieved_data = get_result.unwrap();
        assert!(retrieved_data.is_some());
        assert_eq!(retrieved_data.unwrap(), test_data);
    }

    #[test]
    fn test_update_nonexistent() {
        // Test updating a non-existent value
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::new(1, 2);

        let update_result = item.update(
            |data| {
                assert_eq!(data, None);
                Ok(TestData {
                    value: 456,
                    name: "updated".to_string(),
                })
            },
            &mut env,
        );

        assert!(update_result.is_ok());
        let updated_data = update_result.unwrap();
        assert_eq!(updated_data.value, 456);
        assert_eq!(updated_data.name, "updated");

        // Verify the value was stored
        let get_result = item.may_get(&mut env).unwrap();
        assert!(get_result.is_some());
        assert_eq!(get_result.unwrap(), updated_data);
    }

    #[test]
    fn test_update_existing() {
        // Test updating an existing value
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::new(1, 2);

        // First set a value
        let initial_data = TestData {
            value: 123,
            name: "initial".to_string(),
        };
        item.set(&initial_data, &mut env).unwrap();

        // Now update it
        let update_result = item.update(
            |data| {
                assert!(data.is_some());
                let mut existing = data.unwrap();
                existing.value += 100;
                existing.name = format!("{}_updated", existing.name);
                Ok(existing)
            },
            &mut env,
        );

        assert!(update_result.is_ok());
        let updated_data = update_result.unwrap();
        assert_eq!(updated_data.value, 223);
        assert_eq!(updated_data.name, "initial_updated");

        // Verify the value was stored
        let get_result = item.may_get(&mut env).unwrap();
        assert!(get_result.is_some());
        assert_eq!(get_result.unwrap(), updated_data);
    }

    #[test]
    fn test_update_error_handling() {
        // Test that errors from the update function are properly propagated
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::new(1, 2);

        // Set initial value
        let initial_data = TestData {
            value: 123,
            name: "initial".to_string(),
        };
        item.set(&initial_data, &mut env).unwrap();

        // Update with an error
        let update_result = item.update(
            |_| Err(ErrorCode::new(42)), // Return an error
            &mut env,
        );

        assert!(update_result.is_err());
        assert_eq!(update_result.unwrap_err(), ErrorCode::new(42));

        // Verify the value was not changed
        let get_result = item.may_get(&mut env).unwrap();
        assert!(get_result.is_some());
        assert_eq!(get_result.unwrap(), initial_data);
    }

    #[test]
    fn test_different_prefixes() {
        // Test that items with different prefixes don't interfere with each other
        let item1: Item<TestData> = Item::new(1);
        let item2: Item<TestData> = Item::new(2);
        let mut env = MockEnvironment::new(1, 2);

        // Set values in both items
        let data1 = TestData {
            value: 111,
            name: "item1".to_string(),
        };
        let data2 = TestData {
            value: 222,
            name: "item2".to_string(),
        };

        item1.set(&data1, &mut env).unwrap();
        item2.set(&data2, &mut env).unwrap();

        // Verify they have different values
        let get1 = item1.may_get(&mut env).unwrap().unwrap();
        let get2 = item2.may_get(&mut env).unwrap().unwrap();

        assert_eq!(get1, data1);
        assert_eq!(get2, data2);
        assert_ne!(get1, get2);
    }

    #[test]
    fn test_get_with_environment_error() {
        // Test that errors from the environment are properly propagated during get
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::with_failure();

        let result = item.may_get(&mut env);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 99);
    }

    #[test]
    fn test_set_with_environment_error() {
        // Test that errors from the environment are properly propagated during set
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::with_failure();

        let test_data = TestData {
            value: 123,
            name: "test".to_string(),
        };

        let result = item.set(&test_data, &mut env);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 99);
    }

    #[test]
    fn test_update_with_environment_error() {
        // Test that errors from the environment are properly propagated during update
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::with_failure();

        let result = item.update(
            |_| {
                Ok(TestData {
                    value: 456,
                    name: "updated".to_string(),
                })
            },
            &mut env,
        );

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 99);
    }

    #[test]
    fn test_overwrite_value() {
        // Test overwriting an existing value
        let item: Item<TestData> = Item::new(42);
        let mut env = MockEnvironment::new(1, 2);

        // Set initial value
        let initial_data = TestData {
            value: 123,
            name: "initial".to_string(),
        };
        item.set(&initial_data, &mut env).unwrap();

        // Set a new value
        let new_data = TestData {
            value: 456,
            name: "new".to_string(),
        };
        item.set(&new_data, &mut env).unwrap();

        // Verify the value was overwritten
        let get_result = item.may_get(&mut env).unwrap();
        assert!(get_result.is_some());
        assert_eq!(get_result.unwrap(), new_data);
    }

    #[test]
    fn test_with_different_account_ids() {
        // Test that items with the same prefix but different account IDs don't interfere
        let item: Item<TestData> = Item::new(42);

        // Create two environments with different account IDs
        let mut env1 = MockEnvironment::new(1, 2);
        let mut env2 = MockEnvironment::new(3, 4);

        // Set values in both environments
        let data1 = TestData {
            value: 111,
            name: "env1".to_string(),
        };
        let data2 = TestData {
            value: 222,
            name: "env2".to_string(),
        };

        item.set(&data1, &mut env1).unwrap();
        item.set(&data2, &mut env2).unwrap();

        // Verify they have different values
        let get1 = item.may_get(&mut env1).unwrap().unwrap();
        let get2 = item.may_get(&mut env2).unwrap().unwrap();

        assert_eq!(get1, data1);
        assert_eq!(get2, data2);
        assert_ne!(get1, get2);
    }
}
