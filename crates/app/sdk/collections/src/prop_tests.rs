//! Property-based tests for collections.

// Testing code - determinism requirements do not apply.
#![allow(clippy::disallowed_types)]

use crate::item::Item;
use crate::map::Map;
use crate::mocks::MockEnvironment;
use crate::queue::Queue;
use crate::unordered_map::UnorderedMap;
use crate::vector::Vector;
use crate::{ERR_EMPTY, ERR_NOT_FOUND};
use proptest::prelude::*;
use std::collections::{HashMap, VecDeque};

const MAX_OPS: usize = 32;
const MAX_KEYS: usize = 16;
const DEFAULT_CASES: u32 = 128;
const CI_CASES: u32 = 32;

fn proptest_cases() -> u32 {
    if let Ok(value) = std::env::var("EVOLVE_PROPTEST_CASES") {
        if let Ok(parsed) = value.parse::<u32>() {
            if parsed > 0 {
                return parsed;
            }
        }
    }

    if std::env::var("EVOLVE_CI").is_ok() || std::env::var("CI").is_ok() {
        return CI_CASES;
    }

    DEFAULT_CASES
}

fn proptest_config() -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases: proptest_cases(),
        ..Default::default()
    }
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn prop_map_set_get_remove(key in any::<u64>(), value in any::<u64>()) {
        let map: Map<u64, u64> = Map::new(10);
        let mut env = MockEnvironment::new(1, 2);

        map.set(&key, &value, &mut env).unwrap();
        prop_assert_eq!(map.may_get(&key, &mut env).unwrap(), Some(value));

        map.remove(&key, &mut env).unwrap();
        prop_assert_eq!(map.may_get(&key, &mut env).unwrap(), None);
    }

    #[test]
    fn prop_map_update(key in any::<u64>(), initial in any::<u64>(), updated in any::<u64>()) {
        let map: Map<u64, u64> = Map::new(11);
        let mut env = MockEnvironment::new(1, 2);

        map.set(&key, &initial, &mut env).unwrap();
        let new_value = map.update(&key, |_| Ok(updated), &mut env).unwrap();

        prop_assert_eq!(new_value, updated);
        prop_assert_eq!(map.get(&key, &mut env).unwrap(), updated);
    }

    #[test]
    fn prop_item_set_get_remove(value in any::<u64>()) {
        let item: Item<u64> = Item::new(12);
        let mut env = MockEnvironment::new(1, 2);

        item.set(&value, &mut env).unwrap();
        prop_assert_eq!(item.may_get(&mut env).unwrap(), Some(value));

        item.remove(&mut env).unwrap();
        prop_assert_eq!(item.may_get(&mut env).unwrap(), None);
        prop_assert_eq!(item.get(&mut env).unwrap_err(), ERR_NOT_FOUND);
    }
}

#[derive(Clone, Debug)]
enum VectorOp {
    Push(u64),
    Pop,
}

fn vector_ops_strategy() -> impl Strategy<Value = Vec<VectorOp>> {
    let op = prop_oneof![
        3 => any::<u64>().prop_map(VectorOp::Push),
        1 => Just(VectorOp::Pop),
    ];

    proptest::collection::vec(op, 0..=MAX_OPS)
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn prop_vector_matches_model(ops in vector_ops_strategy()) {
        let vec: Vector<u64> = Vector::new(20, 21);
        let mut env = MockEnvironment::new(1, 2);
        let mut model: Vec<u64> = Vec::new();

        for op in ops {
            match op {
                VectorOp::Push(value) => {
                    vec.push(&value, &mut env).unwrap();
                    model.push(value);
                }
                VectorOp::Pop => {
                    let got = vec.pop(&mut env).unwrap();
                    let expected = model.pop();
                    prop_assert_eq!(got, expected);
                }
            }

            prop_assert_eq!(vec.len(&mut env).unwrap(), model.len() as u64);
        }

        for (index, value) in model.iter().enumerate() {
            prop_assert_eq!(vec.get(index as u64, &mut env).unwrap(), *value);
        }
    }
}

#[derive(Clone, Debug)]
enum QueueOp {
    Push(u64),
    PopFront,
    PopBack,
}

fn queue_ops_strategy() -> impl Strategy<Value = Vec<QueueOp>> {
    let op = prop_oneof![
        3 => any::<u64>().prop_map(QueueOp::Push),
        1 => Just(QueueOp::PopFront),
        1 => Just(QueueOp::PopBack),
    ];

    proptest::collection::vec(op, 0..=MAX_OPS)
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn prop_queue_matches_model(ops in queue_ops_strategy()) {
        let queue: Queue<u64> = Queue::new(30, 31, 32);
        let mut env = MockEnvironment::new(1, 2);
        let mut model: VecDeque<u64> = VecDeque::new();

        for op in ops {
            match op {
                QueueOp::Push(value) => {
                    queue.push_back(&value, &mut env).unwrap();
                    model.push_back(value);
                }
                QueueOp::PopFront => {
                    let result = queue.pop_front(&mut env);
                    match model.pop_front() {
                        Some(expected) => prop_assert_eq!(result.unwrap(), expected),
                        None => prop_assert_eq!(result.unwrap_err(), ERR_EMPTY),
                    }
                }
                QueueOp::PopBack => {
                    let result = queue.pop_back(&mut env);
                    match model.pop_back() {
                        Some(expected) => prop_assert_eq!(result.unwrap(), expected),
                        None => prop_assert_eq!(result.unwrap_err(), ERR_EMPTY),
                    }
                }
            }
        }

        prop_assert_eq!(queue.is_empty(&mut env).unwrap(), model.is_empty());
    }
}

#[derive(Clone, Debug)]
enum UnorderedMapOp {
    Insert { key: u64, value: u64 },
    Remove { key: u64 },
}

fn unordered_map_ops_strategy() -> impl Strategy<Value = Vec<UnorderedMapOp>> {
    let keys: Vec<u64> = (0..MAX_KEYS as u64).collect();

    let insert = (proptest::sample::select(keys.clone()), any::<u64>())
        .prop_map(|(key, value)| UnorderedMapOp::Insert { key, value });
    let remove = proptest::sample::select(keys).prop_map(|key| UnorderedMapOp::Remove { key });

    let op = prop_oneof![3 => insert, 1 => remove];
    proptest::collection::vec(op, 0..=MAX_OPS)
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn prop_unordered_map_matches_model(ops in unordered_map_ops_strategy()) {
        let map: UnorderedMap<u64, u64> = UnorderedMap::new(40, 41, 42, 43);
        let mut env = MockEnvironment::new(1, 2);
        let mut model: HashMap<u64, u64> = HashMap::new();

        for op in ops {
            match op {
                UnorderedMapOp::Insert { key, value } => {
                    let expected = model.insert(key, value);
                    let result = map.insert(&key, &value, &mut env).unwrap();
                    prop_assert_eq!(result, expected);
                }
                UnorderedMapOp::Remove { key } => {
                    let expected = model.remove(&key);
                    let result = map.remove(&key, &mut env).unwrap();
                    prop_assert_eq!(result, expected);
                }
            }

            prop_assert_eq!(map.len(&mut env).unwrap(), model.len() as u64);

            for key in 0..MAX_KEYS as u64 {
                let expected = model.get(&key).copied();
                let actual = map.may_get(&key, &mut env).unwrap();
                prop_assert_eq!(actual, expected);
            }
        }

        let mut actual_pairs: Vec<(u64, u64)> = Vec::new();
        for entry in map.iter(&mut env).unwrap() {
            actual_pairs.push(entry.unwrap());
        }
        actual_pairs.sort_by_key(|(key, _)| *key);

        let mut expected_pairs: Vec<(u64, u64)> = model.into_iter().collect();
        expected_pairs.sort_by_key(|(key, _)| *key);

        prop_assert_eq!(actual_pairs, expected_pairs);
    }
}
