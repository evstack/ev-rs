use crate::encoding::{Decodable, Encodable};
use crate::{InvokableMessage, SdkResult};
use borsh::{BorshDeserialize, BorshSerialize};
use std::borrow::Cow;
use std::io::{Read, Write};

#[derive(Debug, Clone)]
pub struct Message {
    inner: InnerMessage,
}

impl Message {
    pub fn into_bytes(self) -> SdkResult<Vec<u8>> {
        self.inner.into_bytes()
    }

    pub fn as_vec(&self) -> SdkResult<Vec<u8>> {
        self.inner.as_vec()
    }

    pub fn as_bytes(&self) -> SdkResult<&[u8]> {
        self.inner.as_bytes()
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            inner: InnerMessage::OwnedBytes(bytes),
        }
    }

    pub fn len(&self) -> usize {
        match &self.inner {
            InnerMessage::OwnedBytes(bytes) => bytes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.inner {
            InnerMessage::OwnedBytes(bytes) => bytes.is_empty(),
        }
    }
}

#[derive(Debug)]
enum InnerMessage {
    OwnedBytes(Vec<u8>),
}

impl InnerMessage {
    pub(crate) fn as_vec(&self) -> SdkResult<Vec<u8>> {
        match self {
            InnerMessage::OwnedBytes(bytes) => Ok(bytes.clone()),
        }
    }

    pub(crate) fn as_bytes(&self) -> SdkResult<&[u8]> {
        match self {
            InnerMessage::OwnedBytes(bytes) => Ok(bytes),
        }
    }

    pub(crate) fn into_bytes(self) -> SdkResult<Vec<u8>> {
        match self {
            InnerMessage::OwnedBytes(bytes) => Ok(bytes),
        }
    }
}

impl Clone for InnerMessage {
    fn clone(&self) -> Self {
        match self {
            InnerMessage::OwnedBytes(bytes) => InnerMessage::OwnedBytes(bytes.clone()),
        }
    }
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

#[derive(Debug, Clone)]
pub struct InvokeRequest {
    human_name: Cow<'static, str>,
    function: u64,
    message: Message,
}

impl InvokeRequest {
    pub fn new<M: InvokableMessage>(msg: &M) -> SdkResult<Self> {
        Ok(Self {
            human_name: Cow::Borrowed(M::FUNCTION_IDENTIFIER_NAME),
            function: M::FUNCTION_IDENTIFIER,
            message: Message::new(msg)?,
        })
    }
    pub fn new_from_message(human_name: &'static str, function: u64, message: Message) -> Self {
        Self {
            human_name: Cow::Borrowed(human_name),
            function,
            message,
        }
    }

    pub fn function(&self) -> u64 {
        self.function
    }

    pub fn human_name(&self) -> &str {
        self.human_name.as_ref()
    }

    pub fn get<T: Decodable>(&self) -> SdkResult<T> {
        self.message.get()
    }
}

impl BorshSerialize for InvokeRequest {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let name_bytes = self.human_name.as_ref().as_bytes();
        let len: u32 = name_bytes
            .len()
            .try_into()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name too long"))?;
        writer.write_all(&len.to_le_bytes())?;
        writer.write_all(name_bytes)?;
        self.function.serialize(writer)?;
        self.message.serialize(writer)?;
        Ok(())
    }
}

impl BorshDeserialize for InvokeRequest {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let len = u32::deserialize_reader(reader)? as usize;
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        let name = String::from_utf8(buf)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf-8"))?;
        let function = u64::deserialize_reader(reader)?;
        let message = Message::deserialize_reader(reader)?;
        Ok(Self {
            human_name: Cow::Owned(name),
            function,
            message,
        })
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
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
