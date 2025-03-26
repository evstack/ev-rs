use crate::ERR_NOT_FOUND;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageSetRequest,
    STORAGE_ACCOUNT_ID,
};
use evolve_core::{Environment, InvokeRequest, Message, SdkResult};
use std::marker::PhantomData;

pub struct Map<K, V> {
    prefix: u8,
    _kv: PhantomData<(K, V)>,
}

impl<K, V> Map<K, V> {
    pub const fn new(prefix: u8) -> Map<K, V> {
        Map {
            prefix,
            _kv: PhantomData,
        }
    }
}

impl<K, V> Map<K, V>
where
    K: Encodable + Decodable,
    V: Encodable + Decodable,
{
    pub fn set(&self, key: &K, value: &V, backend: &mut dyn Environment) -> SdkResult<()> {
        backend.do_exec(
            STORAGE_ACCOUNT_ID,
            &InvokeRequest::new(&StorageSetRequest {
                key: self.make_key(key)?,
                value: Message::new(value)?,
            })?,
            vec![],
        )?;

        Ok(())
    }

    pub fn get(&self, key: &K, backend: &dyn Environment) -> SdkResult<V> {
        self.may_get(key, backend)?.ok_or(ERR_NOT_FOUND)
    }

    pub fn may_get(&self, key: &K, env: &dyn Environment) -> SdkResult<Option<V>> {
        let bytes_key = self.make_key(key)?;

        let resp = env
            .do_query(
                STORAGE_ACCOUNT_ID,
                &InvokeRequest::new(&StorageGetRequest {
                    account_id: env.whoami(),
                    key: bytes_key,
                })?,
            )?
            .get::<StorageGetResponse>()?;

        match resp.value {
            None => Ok(None),
            Some(v) => Ok(Some(v.get()?)),
        }
    }

    pub fn remove(&self, key: &K, backend: &mut dyn Environment) -> SdkResult<()> {
        backend.do_exec(
            STORAGE_ACCOUNT_ID,
            &InvokeRequest::new(&StorageRemoveRequest {
                key: self.make_key(key)?,
            })?,
            vec![],
        )?;

        Ok(())
    }

    pub fn update(
        &self,
        key: &K,
        update_fn: impl FnOnce(Option<V>) -> SdkResult<V>,
        backend: &mut dyn Environment,
    ) -> SdkResult<V> {
        let old_value = self.may_get(key, backend)?;
        let new_value = update_fn(old_value)?;
        self.set(key, &new_value, backend)?;
        Ok(new_value)
    }

    pub fn make_key(&self, key: &K) -> SdkResult<Vec<u8>> {
        let mut key_bytes = vec![self.prefix];
        key_bytes.extend(key.encode()?);
        Ok(key_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mocks::MockEnvironment;
    use borsh::{BorshDeserialize, BorshSerialize};

    // Test data structure that implements Encodable and Decodable via BorshSerialize and BorshDeserialize
    #[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Clone)]
    struct TestData {
        value: u32,
        name: String,
    }

    #[test]
    fn test_new() {
        // Test that Map::new creates a Map with the correct prefix
        let _map: Map<String, TestData> = Map::new(42);
    }

    #[test]
    fn test_get_nonexistent() {
        // Test getting a value that doesn't exist
        let map: Map<String, TestData> = Map::new(42);
        let env = MockEnvironment::new(1, 2);

        let result = map.may_get(&"test_key".to_string(), &env);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_set_and_get() {
        // Test setting and then getting a value
        let map: Map<String, TestData> = Map::new(42);
        let mut env = MockEnvironment::new(1, 2);

        let test_data = TestData {
            value: 123,
            name: "test".to_string(),
        };

        // Set the value
        let set_result = map.set(&"test_key".to_string(), &test_data, &mut env);
        assert!(set_result.is_ok());

        // Get the value
        let get_result = map.may_get(&"test_key".to_string(), &env);
        assert!(get_result.is_ok());

        let retrieved_data = get_result.unwrap();
        assert!(retrieved_data.is_some());
        assert_eq!(retrieved_data.unwrap(), test_data);
    }

    #[test]
    fn test_update_nonexistent() {
        // Test updating a non-existent value
        let map: Map<String, TestData> = Map::new(42);
        let mut env = MockEnvironment::new(1, 2);

        let update_result = map.update(
            &"test_key".to_string(),
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
        let get_result = map.may_get(&"test_key".to_string(), &env).unwrap();
        assert!(get_result.is_some());
        assert_eq!(get_result.unwrap(), updated_data);
    }

    #[test]
    fn test_update_existing() {
        // Test updating an existing value
        let map: Map<String, TestData> = Map::new(42);
        let mut env = MockEnvironment::new(1, 2);

        // Set initial value
        let initial_data = TestData {
            value: 123,
            name: "initial".to_string(),
        };
        map.set(&"test_key".to_string(), &initial_data, &mut env)
            .unwrap();

        // Update the value
        let update_result = map.update(
            &"test_key".to_string(),
            |data| {
                let mut data = data.unwrap();
                data.value += 100;
                data.name = format!("{}_updated", data.name);
                Ok(data)
            },
            &mut env,
        );

        assert!(update_result.is_ok());
        let updated_data = update_result.unwrap();
        assert_eq!(updated_data.value, 223);
        assert_eq!(updated_data.name, "initial_updated");

        // Verify the value was stored
        let get_result = map.may_get(&"test_key".to_string(), &env).unwrap();
        assert!(get_result.is_some());
        assert_eq!(get_result.unwrap(), updated_data);
    }

    #[test]
    fn test_different_prefixes() {
        // Test that maps with different prefixes don't interfere with each other
        let map1: Map<String, TestData> = Map::new(1);
        let map2: Map<String, TestData> = Map::new(2);
        let mut env = MockEnvironment::new(1, 2);

        // Set values in both maps
        let data1 = TestData {
            value: 111,
            name: "map1".to_string(),
        };
        let data2 = TestData {
            value: 222,
            name: "map2".to_string(),
        };

        map1.set(&"same_key".to_string(), &data1, &mut env).unwrap();
        map2.set(&"same_key".to_string(), &data2, &mut env).unwrap();

        // Verify they have different values
        let get1 = map1
            .may_get(&"same_key".to_string(), &env)
            .unwrap()
            .unwrap();
        let get2 = map2
            .may_get(&"same_key".to_string(), &env)
            .unwrap()
            .unwrap();

        assert_eq!(get1, data1);
        assert_eq!(get2, data2);
        assert_ne!(get1, get2);
    }

    #[test]
    fn test_different_keys() {
        // Test that different keys in the same map don't interfere with each other
        let map: Map<String, TestData> = Map::new(42);
        let mut env = MockEnvironment::new(1, 2);

        // Set values with different keys
        let data1 = TestData {
            value: 111,
            name: "data1".to_string(),
        };
        let data2 = TestData {
            value: 222,
            name: "data2".to_string(),
        };

        map.set(&"key1".to_string(), &data1, &mut env).unwrap();
        map.set(&"key2".to_string(), &data2, &mut env).unwrap();

        // Verify they have different values
        let get1 = map.may_get(&"key1".to_string(), &env).unwrap().unwrap();
        let get2 = map.may_get(&"key2".to_string(), &env).unwrap().unwrap();

        assert_eq!(get1, data1);
        assert_eq!(get2, data2);
        assert_ne!(get1, get2);
    }

    #[test]
    fn test_get_with_environment_error() {
        // Test that errors from the environment are properly propagated during get
        let map: Map<String, TestData> = Map::new(42);
        let env = MockEnvironment::with_failure();

        let result = map.may_get(&"test_key".to_string(), &env);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 99);
    }

    #[test]
    fn test_set_with_environment_error() {
        // Test that errors from the environment are properly propagated during set
        let map: Map<String, TestData> = Map::new(42);
        let mut env = MockEnvironment::with_failure();

        let test_data = TestData {
            value: 123,
            name: "test".to_string(),
        };

        let result = map.set(&"test_key".to_string(), &test_data, &mut env);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 99);
    }

    #[test]
    fn test_update_with_environment_error() {
        // Test that errors from the environment are properly propagated during update
        let map: Map<String, TestData> = Map::new(42);
        let mut env = MockEnvironment::with_failure();

        let result = map.update(
            &"test_key".to_string(),
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
    fn test_make_key() {
        // Test that make_key correctly prefixes the key
        let map: Map<u32, TestData> = Map::new(42);

        let key = 123u32;
        let result = map.make_key(&key);

        assert!(result.is_ok());
        let key_bytes = result.unwrap();

        // The first byte should be the prefix
        assert_eq!(key_bytes[0], 42);

        // The rest should be the encoded key
        let encoded_key = key.encode().unwrap();
        assert_eq!(key_bytes[1..], encoded_key[..]);
    }

    #[test]
    fn test_with_different_account_ids() {
        // Test that maps with the same prefix but different account IDs don't interfere
        let map: Map<String, TestData> = Map::new(42);

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

        map.set(&"same_key".to_string(), &data1, &mut env1).unwrap();
        map.set(&"same_key".to_string(), &data2, &mut env2).unwrap();

        // Verify they have different values
        let get1 = map
            .may_get(&"same_key".to_string(), &env1)
            .unwrap()
            .unwrap();
        let get2 = map
            .may_get(&"same_key".to_string(), &env2)
            .unwrap()
            .unwrap();

        assert_eq!(get1, data1);
        assert_eq!(get2, data2);
        assert_ne!(get1, get2);
    }
}
