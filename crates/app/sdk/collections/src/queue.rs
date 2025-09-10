use crate::item::Item;
use crate::map::Map;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{Environment, EnvironmentQuery, SdkResult};

/// A simple queue with indices [front..back).
///
/// - `front` is the index of the next item to pop from the front.
/// - `back` is the index of the next *unused* slot (the position we push to).
/// - Items are stored in a `Map<u64, V>`.
pub struct Queue<V> {
    front: Item<u64>,
    back: Item<u64>,
    items: Map<u64, V>,
}

impl<V> Queue<V> {
    /// Creates a new queue with specific storage prefixes.
    /// You can ensure they differ from other data structures in your contract.
    pub const fn new(front_prefix: u8, back_prefix: u8, items_prefix: u8) -> Self {
        Self {
            front: Item::new(front_prefix),
            back: Item::new(back_prefix),
            items: Map::new(items_prefix),
        }
    }

    /// Returns the current value of `back`, or 0 if not set yet.
    fn get_back(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u64> {
        Ok(self.back.may_get(env)?.unwrap_or_default())
    }

    /// Returns the current value of `front`, or 0 if not set yet.
    fn get_front(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u64> {
        Ok(self.front.may_get(env)?.unwrap_or_default())
    }
}

impl<V> Queue<V>
where
    V: Encodable + Decodable,
{
    /// Enqueues an item at the back of the queue.
    /// Complexity: O(1)
    pub fn push_back(&self, item: &V, env: &mut dyn Environment) -> SdkResult<()> {
        let back = self.get_back(env)?;

        // Store the item at index `back`
        self.items.set(&back, item, env)?;

        // Move `back` one step
        self.back.set(&(back + 1), env)?;
        Ok(())
    }

    /// Dequeues an item from the front of the queue.
    /// Returns an error `crate::ERR_EMPTY` if the queue is empty.
    /// Complexity: O(1)
    pub fn pop_front(&self, env: &mut dyn Environment) -> SdkResult<V> {
        let front = self.get_front(env)?;
        let back = self.get_back(env)?;

        if front == back {
            return Err(crate::ERR_EMPTY);
        }

        // Retrieve the item at `front`
        let item = self.items.get(&front, env)?;
        // Remove from storage
        self.items.remove(&front, env)?;
        // Move `front` one step
        self.front.set(&(front + 1), env)?;

        Ok(item)
    }

    /// Dequeues an item from the *back* of the queue.
    /// Returns an error `crate::ERR_EMPTY` if the queue is empty.
    /// Complexity: O(1)
    pub fn pop_back(&self, env: &mut dyn Environment) -> SdkResult<V> {
        let front = self.get_front(env)?;
        let back = self.get_back(env)?;

        if back == front {
            return Err(crate::ERR_EMPTY);
        }

        let last_index = back - 1;
        // Retrieve the item at `back - 1`
        let item = self.items.get(&last_index, env)?;
        // Remove from storage
        self.items.remove(&last_index, env)?;
        // Decrement `back`
        self.back.set(&last_index, env)?;

        Ok(item)
    }

    /// Returns `true` if the queue has no items.
    /// Complexity: O(1)
    pub fn is_empty(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<bool> {
        let front = self.get_front(env)?;
        let back = self.get_back(env)?;
        Ok(front == back)
    }
    /// Returns an iterator over the items from front to back.
    ///
    /// - Each `next()` yields `SdkResult<V>`.
    /// - If some storage read fails, the iterator yields an `Err(...)`.
    /// - Once the queue is exhausted, `next()` returns `None`.
    pub fn iter<'a>(&'a self, env: &'a mut dyn EnvironmentQuery) -> SdkResult<QueueIter<'a, V>> {
        let front = self.get_front(env)?;
        let back = self.get_back(env)?;
        Ok(QueueIter {
            queue: self,
            env,
            index: front,
            end: back,
        })
    }
}

/// An iterator over the items in `Queue<V>`.
pub struct QueueIter<'a, V> {
    queue: &'a Queue<V>,
    env: &'a mut dyn EnvironmentQuery,
    index: u64,
    end: u64,
}

impl<V> Iterator for QueueIter<'_, V>
where
    V: Encodable + Decodable,
{
    type Item = SdkResult<V>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.end {
            return None;
        }
        // Attempt to retrieve the item
        let item_result = self.queue.items.may_get(&self.index, self.env);

        self.index += 1;

        match item_result {
            Ok(Some(item)) => Some(Ok(item)),
            Ok(None) => Some(Err(crate::ERR_DATA_CORRUPTION)),
            Err(e) => Some(Err(e)),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mocks::MockEnvironment;
    // You might import your actual MockEnv, but here's a stand-in example:
    use evolve_core::SdkResult;

    #[test]
    fn test_is_empty_initially() -> SdkResult<()> {
        // We'll create a queue of `u64` for simplicity
        let queue: Queue<u64> = Queue::new(0, 1, 2);
        let mut env = MockEnvironment::new(100, 999); // whoami=100, sender=999

        assert!(queue.is_empty(&mut env)?, "Queue should be empty at start");
        Ok(())
    }

    #[test]
    fn test_push_back_and_pop_front() -> SdkResult<()> {
        let queue: Queue<String> = Queue::new(10, 11, 12);
        let mut env = MockEnvironment::new(101, 999);

        // Initially empty
        assert!(queue.is_empty(&mut env)?);

        // Push "Alice"
        queue.push_back(&"Alice".to_string(), &mut env)?;
        assert!(!queue.is_empty(&mut env)?);

        // Push "Bob"
        queue.push_back(&"Bob".to_string(), &mut env)?;

        // Pop from front -> should be "Alice"
        let first = queue.pop_front(&mut env)?;
        assert_eq!(first, "Alice");

        // Pop from front -> should be "Bob"
        let second = queue.pop_front(&mut env)?;
        assert_eq!(second, "Bob");

        // Now empty again
        assert!(queue.is_empty(&mut env)?);

        // Popping from empty => crate::ERR_EMPTY
        let err = queue.pop_front(&mut env).unwrap_err();
        assert_eq!(err.code(), crate::ERR_EMPTY.code());

        Ok(())
    }

    #[test]
    fn test_push_back_and_pop_back() -> SdkResult<()> {
        let queue: Queue<String> = Queue::new(20, 21, 22);
        let mut env = MockEnvironment::new(200, 999);

        // push "A"
        queue.push_back(&"A".to_string(), &mut env)?;
        // push "B"
        queue.push_back(&"B".to_string(), &mut env)?;
        // push "C"
        queue.push_back(&"C".to_string(), &mut env)?;

        // pop_back => should be "C"
        let popped = queue.pop_back(&mut env)?;
        assert_eq!(popped, "C");

        // pop_back => should be "B"
        let popped = queue.pop_back(&mut env)?;
        assert_eq!(popped, "B");

        // pop_back => should be "A"
        let popped = queue.pop_back(&mut env)?;
        assert_eq!(popped, "A");

        // queue is now empty
        assert!(queue.is_empty(&mut env)?);

        // pop_back on empty => crate::ERR_EMPTY
        let err = queue.pop_back(&mut env).unwrap_err();
        assert_eq!(err.code(), crate::ERR_EMPTY.code());

        Ok(())
    }

    #[test]
    fn test_mixed_operations() -> SdkResult<()> {
        let queue: Queue<u32> = Queue::new(30, 31, 32);
        let mut env = MockEnvironment::new(300, 999);

        // push_back 10, 20, 30
        queue.push_back(&10, &mut env)?;
        queue.push_back(&20, &mut env)?;
        queue.push_back(&30, &mut env)?;

        // pop_front => 10
        assert_eq!(queue.pop_front(&mut env)?, 10);
        // queue has [20, 30]

        // pop_back => 30
        assert_eq!(queue.pop_back(&mut env)?, 30);
        // queue has [20]

        // queue not empty
        assert!(!queue.is_empty(&mut env)?);

        // push_back 40, 50
        queue.push_back(&40, &mut env)?;
        queue.push_back(&50, &mut env)?;

        // queue has [20, 40, 50]

        // pop_front => 20
        assert_eq!(queue.pop_front(&mut env)?, 20);

        // pop_front => 40
        assert_eq!(queue.pop_front(&mut env)?, 40);

        // pop_front => 50
        assert_eq!(queue.pop_front(&mut env)?, 50);

        // queue is now empty
        assert!(queue.is_empty(&mut env)?);

        Ok(())
    }

    #[test]
    fn test_large_number_of_items() -> SdkResult<()> {
        let queue: Queue<u64> = Queue::new(40, 41, 42);
        let mut env = MockEnvironment::new(400, 999);

        // push a bunch of items
        for i in 0..100 {
            queue.push_back(&i, &mut env)?;
        }

        // pop them from the back in reverse order
        for i in (0..100).rev() {
            let val = queue.pop_back(&mut env)?;
            assert_eq!(val, i);
        }

        // queue empty now
        assert!(queue.is_empty(&mut env)?);

        // push 1..100 again
        for i in 1..=100 {
            queue.push_back(&i, &mut env)?;
        }

        // pop them from the front in ascending order
        for i in 1..=100 {
            let val = queue.pop_front(&mut env)?;
            assert_eq!(val, i);
        }

        // empty again
        assert!(queue.is_empty(&mut env)?);

        Ok(())
    }
    #[test]
    fn test_iter_over_queue() -> SdkResult<()> {
        let queue: Queue<String> = Queue::new(30, 31, 32);
        let mut env = MockEnvironment::new(4, 400);

        // push some items
        queue.push_back(&"A".to_string(), &mut env)?;
        queue.push_back(&"B".to_string(), &mut env)?;
        queue.push_back(&"C".to_string(), &mut env)?;

        // We'll iterate them from front to back: [A, B, C]
        let mut iter = queue.iter(&mut env)?;
        // 1) next => Ok("A")
        let first = iter.next().unwrap()?;
        assert_eq!(first, "A");
        // 2) next => Ok("B")
        let second = iter.next().unwrap()?;
        assert_eq!(second, "B");
        // 3) next => Ok("C")
        let third = iter.next().unwrap()?;
        assert_eq!(third, "C");
        // 4) next => None
        assert!(iter.next().is_none());

        // Another approach is to collect them all
        let all_items: Result<Vec<_>, _> = queue.iter(&mut env)?.collect();
        let items_vec = all_items?;
        assert_eq!(items_vec, vec!["A", "B", "C"]);

        Ok(())
    }

    #[test]
    fn test_iter_empty_queue() -> SdkResult<()> {
        let queue: Queue<u64> = Queue::new(40, 41, 42);
        let mut env = MockEnvironment::new(5, 500);

        // No items pushed
        let mut iter = queue.iter(&mut env)?;
        assert!(iter.next().is_none(), "No items in an empty queue");

        // We can also confirm that collecting yields an empty vec
        let all: Result<Vec<_>, _> = queue.iter(&mut env)?.collect();
        assert_eq!(all?.len(), 0);

        Ok(())
    }

    #[test]
    fn test_iterator_error_case() -> SdkResult<()> {
        let queue: Queue<u64> = Queue::new(50, 51, 52);
        let mut env = MockEnvironment::new(6, 600);

        // push back a single item
        queue.push_back(&1, &mut env)?;

        // Now let's "manually" remove the item from storage
        // so that the queue index says there's 1 item, but `items` is missing it.
        env.storage_remove(&[52, 0, 0, 0, 0, 0, 0, 0, 0]);
        // ^ We are forging a key remove, for demonstration only.
        //   The actual prefix+backing bytes depends on your encoding scheme.

        // If we try to iterate, we might get an "Item missing from queue" error
        let mut iter = queue.iter(&mut env)?;
        let result = iter.next().unwrap(); // Some(Err(...))

        // Because we forcibly removed the item, we expect error code=404
        let err = result.unwrap_err();
        assert_eq!(
            err,
            crate::ERR_DATA_CORRUPTION,
            "Expected missing item code=404"
        );

        // The next call should yield None since we only had 1 item
        assert!(iter.next().is_none());

        Ok(())
    }
}
