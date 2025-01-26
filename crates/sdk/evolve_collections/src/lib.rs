use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::well_known::{StorageGetRequest, StorageGetResponse, StorageSetRequest, STORAGE_ACCOUNT_ID};
use evolve_core::{Context, InvokeRequest, Invoker, SdkResult};
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
    pub fn set(&self, ctx: &mut Context, backend: &mut dyn Invoker, key: K, value: V) -> SdkResult<()> {
        backend.do_exec(
            ctx,
            STORAGE_ACCOUNT_ID,
            InvokeRequest::try_from(StorageSetRequest {
                key: key.encode()?,
                value: value.encode()?,
            })?,
        )?;

        Ok(())
    }

    pub fn get(&self, ctx: &mut Context, backend: &dyn Invoker, key: K) -> SdkResult<Option<V>> {
        let resp = backend.do_query(
            ctx,
            STORAGE_ACCOUNT_ID,
            InvokeRequest::try_from(StorageGetRequest{
                key: key.encode()?,
            })?
        )?;

        let resp = StorageGetResponse::try_from(resp)?;
        match resp.value {
            None => Ok(None),
            Some(v) => Ok(Some(V::decode(&v)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
    }
}
