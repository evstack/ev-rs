use crate::encoding::{Decodable, Encodable};
use crate::{InvokableMessage, SdkResult};
use borsh::{BorshDeserialize, BorshSerialize};
use std::io::{Read, Write};

#[derive(Debug)]
pub struct Message {
    inner: InnerMessage,
}

#[derive(Debug)]
enum InnerMessage {
    OwnedBytes(Vec<u8>),
}

impl Message {
    pub fn new(encodable: &impl Encodable) -> SdkResult<Message> {
        let bytes = encodable.encode()?;
        Ok(Self {
            inner: InnerMessage::OwnedBytes(bytes),
        })
    }

    pub fn get<T: Decodable>(&self) -> SdkResult<T> {
        match self.inner {
            InnerMessage::OwnedBytes(ref bytes) => T::decode(bytes),
        }
    }
}

impl BorshSerialize for Message {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(
            match self.inner {
                InnerMessage::OwnedBytes(ref bytes) => bytes,
            },
            writer,
        )
    }
}

impl BorshDeserialize for Message {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let bytes: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
        Ok(Self {
            inner: InnerMessage::OwnedBytes(bytes),
        })
    }
}

pub struct InvokeRequest {
    function: u64,
    message: Message,
}

impl InvokeRequest {
    pub fn new<M: InvokableMessage>(msg: &M) -> SdkResult<Self> {
        Ok(Self {
            function: M::FUNCTION_IDENTIFIER,
            message: Message::new(msg)?,
        })
    }
    pub fn new_from_message(function: u64, message: Message) -> Self {
        Self { function, message }
    }

    pub fn function(&self) -> u64 {
        self.function
    }

    pub fn get<T: Decodable>(&self) -> SdkResult<T> {
        self.message.get()
    }
}

pub struct InvokeResponse {
    message: Message,
}

impl InvokeResponse {
    pub fn into_inner(self) -> Message {
        self.message
    }
}

impl InvokeResponse {
    pub fn new_from_message(message: Message) -> Self {
        Self { message }
    }

    pub fn new<T: Encodable>(message: &T) -> SdkResult<Self> {
        Ok(Self {
            message: Message::new(message)?,
        })
    }

    pub fn get<T: Decodable>(&self) -> SdkResult<T> {
        self.message.get()
    }
}
