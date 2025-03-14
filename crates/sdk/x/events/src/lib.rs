use evolve_core::encoding::Encodable;
use evolve_core::events_api::{EmitEventRequest, EVENT_HANDLER_ACCOUNT_ID};
use evolve_core::{Environment, InvokeRequest, Message, SdkResult};

#[derive(Default)]
pub struct EventsEmitter;

impl EventsEmitter {
    pub const fn new() -> Self {
        EventsEmitter
    }
    pub fn emit_event<T: Encodable>(
        &self,
        name: String,
        event: T,
        env: &mut dyn Environment,
    ) -> SdkResult<()> {
        env.do_exec(
            EVENT_HANDLER_ACCOUNT_ID,
            &InvokeRequest::new(&EmitEventRequest {
                name,
                contents: Message::new(&event)?,
            })?,
            vec![],
        )?;
        Ok(())
    }
}
