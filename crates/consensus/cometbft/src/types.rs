use evolve_core::events_api::Event;
use evolve_core::ErrorCode;
use evolve_server_core::{Block, TxDecoder};
use evolve_stf::results::BlockResult;
use std::num::NonZeroU32;
use tendermint::abci::types::{CommitInfo, ExecTxResult, Misbehavior};
use tendermint::abci::{Event as AbciEvent, EventAttribute};

/// This is the metadata for each transaction index in the original incoming list.
/// If decoding was successful, it stores an index pointing into `txs_ok`.
/// If decoding failed, it stores the error message.
pub enum Decoded {
    Ok(usize),
    Err(ErrorCode),
}

pub struct TendermintBlock<T> {
    pub txs_ok: Vec<T>,
    pub decode_history: Vec<Decoded>,
    pub decided_last_commit: CommitInfo,
    pub misbehaviour: Vec<Misbehavior>,
    pub hash: Vec<u8>,
    pub time_unix_ms: u64,
    pub next_validators_hash: Vec<u8>,
    pub proposer_address: Vec<u8>,
    pub height: u64,
}

impl<T> TendermintBlock<T> {
    pub fn from_request_finalize_block(
        decoder: &impl TxDecoder<T>,
        value: tendermint::abci::request::FinalizeBlock,
    ) -> TendermintBlock<T> {
        let mut txs_ok = Vec::new();
        let mut decode_history = Vec::new();

        for tx_bytes in value.txs {
            match decoder.decode(&mut tx_bytes.as_ref()) {
                Ok(tx) => {
                    let idx = txs_ok.len();
                    txs_ok.push(tx);
                    decode_history.push(Decoded::Ok(idx));
                }
                Err(e) => {
                    decode_history.push(Decoded::Err(e));
                }
            }
        }

        TendermintBlock {
            txs_ok,
            decode_history,
            decided_last_commit: value.decided_last_commit,
            misbehaviour: value.misbehavior,
            hash: value.hash.into(),
            time_unix_ms: (value.time.unix_timestamp_nanos() / 1_000_000) as u64,
            next_validators_hash: value.next_validators_hash.into(),
            proposer_address: vec![],
            height: value.height.value(),
        }
    }

    pub fn extract_tx_results(
        self: &mut TendermintBlock<T>,
        mut stf_block_results: BlockResult,
    ) -> Vec<ExecTxResult> {
        let decode_history = core::mem::take(&mut self.decode_history);
        let mut txs_results = Vec::with_capacity(decode_history.len());
        let mut stf_tx_results = core::mem::take(&mut stf_block_results.tx_results)
            .into_iter()
            .rev();

        for decode_status in decode_history {
            match decode_status {
                Decoded::Ok(_) => {
                    let mut stf_tx_result = stf_tx_results.next().unwrap();
                    txs_results.push(ExecTxResult {
                        code: tendermint::abci::Code::Ok,
                        data: Default::default(),
                        log: "".to_string(),
                        info: "".to_string(),
                        gas_wanted: stf_tx_result.gas_used.try_into().unwrap(), // TODO
                        gas_used: stf_tx_result.gas_used.try_into().unwrap(),
                        events: core::mem::take(&mut stf_tx_result.events)
                            .into_iter()
                            .map(evolve_event_into_comet_event)
                            .collect(),
                        codespace: "".to_string(),
                    })
                }
                Decoded::Err(e) => txs_results.push(ExecTxResult {
                    code: tendermint::abci::Code::Err(NonZeroU32::new(1).unwrap()),
                    data: Default::default(),
                    log: "decoding_failed".to_string(),
                    info: e.code().to_string(),
                    gas_wanted: 0,
                    gas_used: 0,
                    events: vec![],
                    codespace: "consensus".to_string(),
                }),
            }
        }

        txs_results
    }
}

pub(crate) fn extract_begin_end_block_events(result: &mut BlockResult) -> Vec<AbciEvent> {
    // Pre-allocate only once based on the total number of events
    let capacity = result.begin_block_events.len() + result.end_block_events.len();
    let mut all_events = Vec::with_capacity(capacity);

    // Extract begin_block events
    let begin_block_events = core::mem::take(&mut result.begin_block_events);
    all_events.extend(events_into_abci(begin_block_events, "begin_block"));

    // Extract end_block events
    let end_block_events = core::mem::take(&mut result.end_block_events);
    all_events.extend(events_into_abci(end_block_events, "end_block"));

    all_events
}

fn events_into_abci<I>(events: I, scope: &str) -> Vec<AbciEvent>
where
    I: IntoIterator<Item = Event>,
{
    events
        .into_iter()
        .map(|evt| AbciEvent {
            kind: evt.name,
            attributes: vec![
                make_attribute("scope", scope),
                make_attribute(
                    "contents",
                    evt.contents
                        .into_bytes()
                        .expect("unable to convert event to bytes"),
                ),
            ],
        })
        .collect()
}

fn make_attribute<K, V>(key: K, value: V) -> EventAttribute
where
    K: Into<String>,
    V: AsRef<[u8]>,
{
    // Base64-encode the value, ensuring it's a valid UTF-8 string
    let encoded_value = base64::encode(value.as_ref());

    EventAttribute::V037(tendermint::abci::v0_37::EventAttribute {
        key: key.into(),
        value: encoded_value,
        index: false,
    })
}

impl<T> Block<T> for TendermintBlock<T> {
    fn height(&self) -> u64 {
        self.height
    }

    /// Return only the *successfully decoded* transactions, as a contiguous slice.
    fn txs(&self) -> &[T] {
        &self.txs_ok
    }
}

fn evolve_event_into_comet_event(
    _event: evolve_core::events_api::Event,
) -> tendermint::abci::Event {
    todo!("impl")
}

#[cfg(test)]
mod test {
    use super::*;
    use evolve_core::{ErrorCode, SdkResult};
    use tendermint::block::Height;
    use tendermint::{abci, Time};

    struct MyTx;

    struct MyDecoder;
    impl TxDecoder<MyTx> for MyDecoder {
        fn decode(&self, bytes: &mut &[u8]) -> SdkResult<MyTx> {
            if bytes.is_empty() {
                Err(ErrorCode::new(0, "empty bytes"))
            } else {
                Ok(MyTx)
            }
        }
    }

    #[test]
    fn test_decode() {
        let valid_tx: Vec<u8> = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let invalid_tx: Vec<u8> = vec![];
        let request = abci::request::FinalizeBlock {
            txs: vec![valid_tx.clone().into(), invalid_tx.clone().into()],
            decided_last_commit: CommitInfo {
                round: Default::default(),
                votes: vec![],
            },
            misbehavior: vec![],
            hash: Default::default(),
            time: Time::from_unix_timestamp(1, 1).unwrap(),
            next_validators_hash: Default::default(),
            proposer_address: tendermint::account::Id::new([
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ]),
            height: Height::try_from(10u64).unwrap(),
        };

        let block = TendermintBlock::from_request_finalize_block(&MyDecoder, request);

        // We should have 1 successful decode, 1 error
        assert_eq!(block.txs_ok.len(), 1);
        assert_eq!(block.decode_history.len(), 2);

        // The Block trait's txs() method returns just the successes:
        assert_eq!(block.txs().len(), 1);
    }
}
