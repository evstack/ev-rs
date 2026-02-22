use crate::item::Item;
use crate::map::Map;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{Environment, EnvironmentQuery, SdkResult};

pub struct Vector<V> {
    index: Item<u64>,
    pub(crate) elems: Map<u64, V>,
}

impl<V> Vector<V> {
    pub const fn new(index_prefix: u8, elems_prefix: u8) -> Self {
        Self {
            index: Item::new(index_prefix),
            elems: Map::new(elems_prefix),
        }
    }
}

impl<V> Vector<V>
where
    V: Encodable + Decodable,
{
    pub fn may_get(&self, index: u64, backend: &mut dyn EnvironmentQuery) -> SdkResult<Option<V>> {
        self.elems.may_get(&index, backend)
    }

    pub fn get(&self, index: u64, backend: &mut dyn EnvironmentQuery) -> SdkResult<V> {
        self.elems.get(&index, backend)
    }

    pub fn push(&self, item: &V, backend: &mut dyn Environment) -> SdkResult<()> {
        let index = self.index.may_get(backend)?.unwrap_or(0);
        self.elems.set(&index, item, backend)?;
        self.index.set(&(index + 1), backend)?;
        Ok(())
    }
    pub fn extend<I: Iterator<Item = V>>(
        &self,
        iter: I,
        backend: &mut dyn Environment,
    ) -> SdkResult<()> {
        let mut index = self.index.may_get(backend)?.unwrap_or(0);
        for item in iter {
            self.elems.set(&index, &item, backend)?;
            index += 1;
        }
        self.index.set(&index, backend)?;
        Ok(())
    }

    pub fn pop(&self, backend: &mut dyn Environment) -> SdkResult<Option<V>> {
        match self.index.may_get(backend)? {
            Some(index) => {
                let value = self.elems.get(&(index - 1), backend)?;
                if index == 1 {
                    self.index.remove(backend)?;
                } else {
                    self.index.set(&(index - 1), backend)?;
                }
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Returns the current length of the `Vector`,
    /// which is how many items have been pushed so far (minus pops).
    pub fn len(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u64> {
        // `Item::may_get(env)` returns an `Option<u64>`:
        //   - `Some(n)` if we have a stored count,
        //   - `None` if no count is stored (meaning zero).
        Ok(self.index.may_get(env)?.unwrap_or(0))
    }

    /// Returns an iterator over the elements in this `Vector`.
    /// Each `next()` call does a storage read to fetch the item at the current index.
    pub fn iter<'a>(&'a self, env: &'a mut dyn EnvironmentQuery) -> SdkResult<VectorIter<'a, V>> {
        let length = self.len(env)?;
        Ok(VectorIter {
            vector: self,
            env,
            index: 0,
            length,
        })
    }
}

/// An iterator over the values stored in a `Vector<V>`.
/// Each `next()` will yield a `SdkResult<V>`, since storage reads can fail.
pub struct VectorIter<'a, V> {
    vector: &'a Vector<V>,
    env: &'a mut dyn EnvironmentQuery,
    index: u64,
    length: u64,
}

impl<V> Iterator for VectorIter<'_, V>
where
    V: Encodable + Decodable,
{
    type Item = SdkResult<V>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.length {
            return None;
        }
        let result = self.vector.get(self.index, self.env);
        self.index += 1;
        Some(result)
    }
}

#[cfg(test)]
mod vector_tests {
    use super::*;
    // Make sure Vector, Environment, etc., are in scope or adjust accordingly
    use crate::mocks::MockEnvironment;
    use borsh::{BorshDeserialize, BorshSerialize};

    // -------------------------------------
    // A simple data type for use in tests
    // -------------------------------------
    #[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Clone)]
    struct TestData {
        value: u32,
        name: String,
    }

    // ------------------------------------
    // Tests for the Vector<V> functionality
    // ------------------------------------

    #[test]
    fn test_may_get_nonexistent() {
        // Vector is empty -> any index should return None
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::new(1, 2);

        assert_eq!(vec.may_get(0, &mut env).unwrap(), None);
        assert_eq!(vec.may_get(5, &mut env).unwrap(), None);
    }

    #[test]
    fn test_push_and_may_get() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::new(1, 2);

        let item1 = TestData {
            value: 100,
            name: "item1".to_string(),
        };
        let item2 = TestData {
            value: 200,
            name: "item2".to_string(),
        };

        // Push items
        vec.push(&item1, &mut env).unwrap();
        vec.push(&item2, &mut env).unwrap();

        // may_get should find them at indices 0 and 1 (assuming push sets them in ascending order)
        assert_eq!(vec.may_get(0, &mut env).unwrap(), Some(item1.clone()));
        assert_eq!(vec.may_get(1, &mut env).unwrap(), Some(item2.clone()));

        // Index 2 is beyond the last pushed item
        assert_eq!(vec.may_get(2, &mut env).unwrap(), None);
    }

    #[test]
    fn test_get_existing() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::new(1, 2);

        let item = TestData {
            value: 999,
            name: "test".to_string(),
        };

        // Push a single item
        vec.push(&item, &mut env).unwrap();

        // get(0) should succeed
        let retrieved = vec.get(0, &mut env).unwrap();
        assert_eq!(retrieved, item);

        // get(1) should fail (not found)
        let err = vec.get(1, &mut env).err().unwrap();
        // Adjust if you have a custom "Not Found" error code
        assert_eq!(err.code(), crate::ERR_NOT_FOUND.code());
    }

    #[test]
    fn test_push_and_pop_single_item() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::new(1, 2);

        let item = TestData {
            value: 1,
            name: "pop_me".to_string(),
        };

        // Push
        vec.push(&item, &mut env).unwrap();

        // Pop (note: current pop code has an off-by-one issue if used as-is)
        let popped = vec.pop(&mut env).unwrap();
        assert_eq!(popped, Some(item));

        // Another pop -> None
        assert_eq!(vec.pop(&mut env).unwrap(), None);
    }

    #[test]
    fn test_push_and_pop_multiple_items() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::new(1, 2);

        let item1 = TestData {
            value: 10,
            name: "item1".to_string(),
        };
        let item2 = TestData {
            value: 20,
            name: "item2".to_string(),
        };
        let item3 = TestData {
            value: 30,
            name: "item3".to_string(),
        };

        // Push in the order 1, 2, 3
        vec.push(&item1, &mut env).unwrap();
        vec.push(&item2, &mut env).unwrap();
        vec.push(&item3, &mut env).unwrap();

        // Pop them (LIFO if the code is correct)
        let pop1 = vec.pop(&mut env).unwrap();
        let pop2 = vec.pop(&mut env).unwrap();
        let pop3 = vec.pop(&mut env).unwrap();

        // Last item pushed should come out first
        assert_eq!(pop1, Some(item3));
        assert_eq!(pop2, Some(item2));
        assert_eq!(pop3, Some(item1));

        // Should be empty now
        assert_eq!(vec.pop(&mut env).unwrap(), None);
    }

    #[test]
    fn test_extend() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::new(1, 2);

        let item1 = TestData {
            value: 101,
            name: "extend1".to_string(),
        };
        let item2 = TestData {
            value: 202,
            name: "extend2".to_string(),
        };
        let item3 = TestData {
            value: 303,
            name: "extend3".to_string(),
        };

        // Use extend with an iterator of items
        let items = vec![item1.clone(), item2.clone(), item3.clone()];
        vec.extend(items.into_iter(), &mut env).unwrap();

        // Check we can retrieve them at indices 0..2
        assert_eq!(vec.get(0, &mut env).unwrap(), item1);
        assert_eq!(vec.get(1, &mut env).unwrap(), item2);
        assert_eq!(vec.get(2, &mut env).unwrap(), item3);
        assert!(vec.may_get(3, &mut env).unwrap().is_none());
    }

    #[test]
    fn test_push_environment_error() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::with_failure();

        let item = TestData {
            value: 777,
            name: "error".to_string(),
        };

        // Attempting to push should fail with code 99
        let err = vec.push(&item, &mut env).err().unwrap();
        assert_eq!(err.code(), 99);
    }

    #[test]
    fn test_pop_environment_error() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::with_failure();

        // Attempting to pop should fail
        let err = vec.pop(&mut env).err().unwrap();
        assert_eq!(err.code(), 99);
    }

    #[test]
    fn test_get_environment_error() {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::with_failure();

        // Attempting to get(0) should fail
        let err = vec.get(0, &mut env).err().unwrap();
        assert_eq!(err.code(), 99);
    }

    #[test]
    fn test_with_different_prefixes() {
        // Two vectors with different prefixes should not interfere in storage
        let vec1: Vector<TestData> = Vector::new(1, 2);
        let vec2: Vector<TestData> = Vector::new(3, 4);
        let mut env = MockEnvironment::new(1, 2);

        let item1 = TestData {
            value: 111,
            name: "vec1".to_string(),
        };
        let item2 = TestData {
            value: 222,
            name: "vec2".to_string(),
        };

        vec1.push(&item1, &mut env).unwrap();
        vec2.push(&item2, &mut env).unwrap();

        // Ensure they do not clash
        assert_eq!(vec1.may_get(0, &mut env).unwrap(), Some(item1));
        assert_eq!(vec2.may_get(0, &mut env).unwrap(), Some(item2));
    }

    #[test]
    fn test_with_different_account_ids() {
        // Same prefix, but different account IDs
        // Our MockEnvironment uses whoami() for different storages, so they won't conflict.
        let vec: Vector<TestData> = Vector::new(10, 20);

        let mut env1 = MockEnvironment::new(1, 2);
        let mut env2 = MockEnvironment::new(3, 4);

        let data1 = TestData {
            value: 101,
            name: "env1".to_string(),
        };
        let data2 = TestData {
            value: 202,
            name: "env2".to_string(),
        };

        // Push an item in each environment
        vec.push(&data1, &mut env1).unwrap();
        vec.push(&data2, &mut env2).unwrap();

        // Each environment's storage is separate
        assert_eq!(vec.may_get(0, &mut env1).unwrap(), Some(data1));
        assert_eq!(vec.may_get(0, &mut env2).unwrap(), Some(data2));
    }

    #[test]
    fn test_vector_iter() -> SdkResult<()> {
        let vec: Vector<TestData> = Vector::new(10, 20);
        let mut env = MockEnvironment::new(1, 2);

        // Push a few items
        vec.push(
            &TestData {
                value: 100,
                name: "".to_string(),
            },
            &mut env,
        )?;
        vec.push(
            &TestData {
                value: 200,
                name: "".to_string(),
            },
            &mut env,
        )?;
        vec.push(
            &TestData {
                value: 300,
                name: "".to_string(),
            },
            &mut env,
        )?;

        // Use the iter() method
        let mut iter = vec.iter(&mut env)?;

        // Because `next()` returns `Option<SdkResult<V>>`,
        // we handle it accordingly:
        assert_eq!(iter.next().unwrap()?.value, 100);
        assert_eq!(iter.next().unwrap()?.value, 200);
        assert_eq!(iter.next().unwrap()?.value, 300);
        // No more items -> `None`
        assert!(iter.next().is_none());

        Ok(())
    }
}
