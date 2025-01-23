pub type ErrorCode = u64;

pub struct AccountId (u128);

enum InnerMessage {
    OwnedBytes(Vec<u8>),
}

/// Defines a message, the internals of this type are hidden such that we can improve them later
/// for performance.
pub struct Message {
    inner: InnerMessage
}

impl From<Vec<u8>> for Message {
    fn from(bytes: Vec<u8>) -> Self {
        Message{
            inner: InnerMessage::OwnedBytes(bytes)
        }
    }
}

/// Defines a request to invoke a method in an account.
pub struct InvokeRequest {
    /// Defines the identifier of the function.
    function_identifier: u64,
    /// Defines the message argument of the function.
    message: Message,
}

impl InvokeRequest {
    pub fn new(function_identifier: u64, message: Message) -> Self {
        InvokeRequest{
            function_identifier,
            message,
        }
    }
}

/// Defines the response of an [`InvokeRequest`]
pub struct InvokeResponse {
    response: Result<Message, ErrorCode>,
}


impl InvokeResponse {
    pub fn new(response: Result<Message, ErrorCode>) -> Self {
        InvokeResponse {
            response,
        }
    }

    pub fn new_error(err_code: ErrorCode) -> Self {
        Self::new(Err(err_code))
    }

    pub fn new_response(response: Result<Message, ErrorCode>) -> Self {
        InvokeResponse::new(response)
    }
}

/// Defines the execution context.
pub struct Context {
    whoami: AccountId,
}

/// Defines some arbitrary code that can handle account execution logic.
pub trait AccountCode {
    fn init(ctx: &mut Context, request: InvokeRequest) -> InvokeResponse;
    fn execute(ctx: &mut Context, request: InvokeRequest) -> InvokeResponse;
    fn query(ctx: &Context, request: InvokeRequest) -> InvokeResponse;
}