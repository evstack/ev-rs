use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use evolve_fungible_asset::TransferMsg;
use evolve_simulator::SimConfig;
use evolve_testapp::block::TestBlock;
use evolve_testapp::sim_testing::SimTestApp;
use evolve_testapp::TestTx;

fn make_transfer_block(
    height: u64,
    tx_count: usize,
    atom_id: evolve_core::AccountId,
    alice: evolve_core::AccountId,
    bob: evolve_core::AccountId,
) -> TestBlock<TestTx> {
    let mut txs = Vec::with_capacity(tx_count);
    for i in 0..tx_count {
        let (sender, recipient) = if i % 2 == 0 {
            (alice, bob)
        } else {
            (bob, alice)
        };
        let request = evolve_core::InvokeRequest::new(&TransferMsg {
            to: recipient,
            amount: 1,
        })
        .expect("request");
        txs.push(TestTx {
            sender,
            recipient: atom_id,
            request,
            gas_limit: 100_000,
            funds: Vec::new(),
        });
    }

    TestBlock::new(height, txs)
}

fn bench_apply_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("stf_apply_block");
    for tx_count in [10usize, 100, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(tx_count), &tx_count, |b, &n| {
            b.iter_batched(
                || {
                    let app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let height = app.simulator().time().block_height();
                    let block =
                        make_transfer_block(height, n, accounts.atom, accounts.alice, accounts.bob);
                    (app, block)
                },
                |(mut app, block)| {
                    app.apply_block(&block);
                    app.next_block();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_apply_block);
criterion_main!(benches);
