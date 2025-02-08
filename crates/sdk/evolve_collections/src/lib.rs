use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::well_known::{
    StorageGetRequest, StorageGetResponse, StorageSetRequest, STORAGE_ACCOUNT_ID,
};
use evolve_core::{Environment, InvokeRequest, SdkResult};
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
            InvokeRequest::try_from(StorageSetRequest {
                key: self.make_key(key)?,
                value: value.encode()?,
            })?,
        )?;

        Ok(())
    }

    pub fn get(&self, key: &K, backend: &dyn Environment) -> SdkResult<Option<V>> {
        let resp = backend.do_query(
            STORAGE_ACCOUNT_ID,
            InvokeRequest::try_from(StorageGetRequest {
                account_id: backend.whoami(),
                key: self.make_key(key)?,
            })?,
        )?;

        let resp = StorageGetResponse::try_from(resp)?;
        match resp.value {
            None => Ok(None),
            Some(v) => Ok(Some(V::decode(&v)?)),
        }
    }

    pub fn update(
        &self,
        key: &K,
        update_fn: impl FnOnce(Option<V>) -> SdkResult<V>,
        backend: &mut dyn Environment,
    ) -> SdkResult<V> {
        let old_value = self.get(key, backend)?;
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
    pub fn get(&self, backend: &dyn Environment) -> SdkResult<Option<V>> {
        self.0.get(&(), backend)
    }

    pub fn set(&self, value: &V, backend: &mut dyn Environment) -> SdkResult<()> {
        self.0.set(&(), value, backend)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
