use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::future::FutureExt;
use tower::{Service, ServiceBuilder};

use tower_abci::BoxError;

use crate::consensus::Consensus;
use crate::types::TendermintBlock;
use evolve_server_core::{
    AccountsCodeStorage, BeginBlocker, EndBlocker, PostTxExecution, Transaction, TxDecoder,
    TxValidator, WritableKV,
};
use evolve_testapp::install_account_codes;
use evolve_testing::server_mocks::AccountStorageMock;
use tendermint::v0_38::abci::{response, Request, Response};
use tower_abci::v038::{split, Server};

impl<A, T, D, Bb, Eb, TxVal, Pt, S> Service<Request> for Consensus<A, T, D, Bb, Eb, TxVal, Pt, S>
where
    A: AccountsCodeStorage,
    T: Transaction,
    D: TxDecoder<T>,
    Bb: BeginBlocker<TendermintBlock<T>>,
    Eb: EndBlocker,
    TxVal: TxValidator<T>,
    Pt: PostTxExecution<T>,
    S: WritableKV,
{
    type Response = Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Response, BoxError>> + Send + 'static>>;

    fn poll_ready(&mut self, _ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        log::debug!("got call: {:?}", req);
        let rsp = match req {
            Request::Echo(req) => Response::Echo(response::Echo {
                message: req.message,
            }),
            Request::PrepareProposal(req) => {
                Response::PrepareProposal(response::PrepareProposal { txs: req.txs })
            }
            Request::FinalizeBlock(block) => Response::FinalizeBlock(self.finalize_block(block)),
            Request::Info(_) => Response::Info(self.info()),
            Request::Query(query) => Response::Query(self.query(query)),
            Request::Commit => Response::Commit(self.commit()),
            Request::ProcessProposal(_) => {
                Response::ProcessProposal(response::ProcessProposal::Accept)
            }
            Request::ExtendVote(v) => Response::ExtendVote(self.extend_vote(v)),
            Request::VerifyVoteExtension(v) => Response::VerifyVoteExtension(self.verify_vote(v)),

            Request::Flush => Response::Flush,
            Request::InitChain(init_chain) => Response::InitChain(self.init_chain(init_chain)),
            Request::CheckTx(_) => Response::CheckTx(Default::default()),
            Request::ListSnapshots => Response::ListSnapshots(Default::default()),
            Request::OfferSnapshot(_) => Response::OfferSnapshot(Default::default()),
            Request::LoadSnapshotChunk(_) => Response::LoadSnapshotChunk(Default::default()),
            Request::ApplySnapshotChunk(_) => Response::ApplySnapshotChunk(Default::default()),
        };
        log::debug!("response: {:?}", rsp);
        async move { Ok(rsp) }.boxed()
    }
}

pub async fn start_server<A, T, D, Bb, Eb, TxVal, Pt, S>(
    endpoint: String,
    cns_impl: Consensus<A, T, D, Bb, Eb, TxVal, Pt, S>,
) where
    A: AccountsCodeStorage + Send + 'static,
    T: Transaction + Send + 'static,
    D: TxDecoder<T> + Send + 'static,
    Bb: BeginBlocker<TendermintBlock<T>> + Send + 'static,
    Eb: EndBlocker + Send + 'static,
    TxVal: TxValidator<T> + Send + 'static,
    Pt: PostTxExecution<T> + Send + 'static,
    S: WritableKV + Send + 'static,
{
    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    // Split it into components.
    let (consensus, mempool, snapshot, info) = split::service(cns_impl, 1);

    // Hand those components to the ABCI server, but customize request behavior
    // for each category -- for instance, apply load-shedding only to mempool
    // and info requests, but not to consensus requests.
    // Note that this example use synchronous execution in `Service::call`, wrapping the end result in a ready future.
    // If `Service::call` did the actual request handling inside the `async` block as well, then the `consensus` service
    // below should be wrapped with a `ServiceBuilder::concurrency_limit` to avoid any unintended reordering of message effects.
    let server_builder = Server::builder()
        .consensus(consensus)
        .snapshot(snapshot)
        .mempool(
            ServiceBuilder::new()
                .load_shed()
                .buffer(10)
                .service(mempool),
        )
        .info(
            ServiceBuilder::new()
                .load_shed()
                .buffer(100)
                .rate_limit(50, std::time::Duration::from_secs(1))
                .service(info),
        );

    server_builder
        .finish()
        .unwrap()
        .listen_tcp(endpoint)
        .await
        .unwrap();
}
