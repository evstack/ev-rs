use crate::Stf;
use evolve_core::{AccountId, Message};
use evolve_server_core::Transaction;

pub(crate) struct MockTx;

impl Transaction for MockTx {
    fn sender(&self) -> AccountId {
        todo!()
    }

    fn recipient(&self) -> AccountId {
        todo!()
    }

    fn message(&self) -> Message {
        todo!()
    }

    fn gas_limit(&self) -> u64 {
        todo!()
    }
}

pub(crate) type TestStf = Stf<MockTx>;
