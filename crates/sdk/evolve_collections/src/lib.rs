use std::marker::PhantomData;
use evolve_core::{Invoker, SdkResult};

pub struct Map<K, V> {
    prefix: u8,
    _kv: PhantomData<(K, V)>
}

impl<K, V> Map<K, V> {
    pub const fn new(prefix: u8) -> Map<K, V> {
        Map { prefix, _kv: PhantomData }
    }
}

impl<K, V> Map<K, V> {
    fn set(&self, backend: &mut dyn Invoker, key: K, value: V) -> SdkResult<()> {
        todo!()
    }

    fn get(&self, backend: &dyn Invoker, key: K) -> SdkResult<V> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
