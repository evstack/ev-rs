use borsh::{BorshDeserialize, BorshSerialize};
use crate::{ErrorCode, Message, SdkResult, ERR_ENCODING};

pub trait Encodable: Sized {
    fn encode(&self) -> SdkResult<Vec<u8>>;
}

pub trait Decodable: Sized {
    fn decode(bytes: &[u8]) -> SdkResult<Self>;
}

// TODO: fast hack into encoding

impl<S: BorshSerialize> Encodable for S {
    fn encode(&self) -> SdkResult<Vec<u8>> {
        borsh::to_vec(self).map_err(|_| ERR_ENCODING)
    }
}

impl<S: BorshDeserialize> Decodable for S {
    fn decode(bytes: &[u8]) -> SdkResult<Self> {
        borsh::from_slice(bytes).map_err(|_| ERR_ENCODING)
    }
}
