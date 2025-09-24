use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use evolve_debugger::{StateSnapshot, TraceBuilder};
use evolve_fungible_asset::TransferMsg;
use evolve_server::Block;
use evolve_simulator::SimConfig;
use evolve_testapp::sim_testing::SimTestApp;
use evolve_testapp::TestTx;

fn make_transfer_block(
    height: u64,
    tx_count: usize,
    atom_id: evolve_core::AccountId,
    alice: evolve_core::AccountId,
    bob: evolve_core::AccountId,
) -> Block<TestTx> {
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

    Block::for_testing(height, txs)
}

fn make_cold_transfer_block(
    height: u64,
    tx_count: usize,
    atom_id: evolve_core::AccountId,
    accounts: &[evolve_core::AccountId],
) -> Block<TestTx> {
    let mut txs = Vec::with_capacity(tx_count);
    let count = accounts.len();
    for i in 0..tx_count {
        let sender = accounts[i % count];
        let recipient = accounts[(i + 1) % count];
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

    Block::for_testing(height, txs)
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

fn bench_apply_block_with_trace(c: &mut Criterion) {
    let mut group = c.benchmark_group("stf_apply_block_trace");
    for tx_count in [10usize, 100, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(tx_count), &tx_count, |b, &n| {
            b.iter_batched(
                || {
                    let app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let height = app.simulator().time().block_height();
                    let block =
                        make_transfer_block(height, n, accounts.atom, accounts.alice, accounts.bob);
                    let snapshot = StateSnapshot::from_data(
                        app.simulator().storage().snapshot().data,
                        app.simulator().time().block_height(),
                        app.simulator().time().now_ms(),
                    );
                    let builder = TraceBuilder::new(app.simulator().seed_info().seed, snapshot);
                    (app, block, builder)
                },
                |(mut app, block, mut builder)| {
                    app.apply_block_with_trace(&block, &mut builder);
                    app.next_block();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_apply_block_hot_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("stf_apply_block_access");
    for tx_count in [100usize, 500] {
        group.bench_with_input(BenchmarkId::new("hot", tx_count), &tx_count, |b, &n| {
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

        group.bench_with_input(BenchmarkId::new("cold", tx_count), &tx_count, |b, &n| {
            b.iter_batched(
                || {
                    let mut app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let mut participants = Vec::with_capacity(256);
                    for _ in 0..256 {
                        let account = app.create_eoa();
                        app.mint_atom(account, 1000);
                        participants.push(account);
                    }
                    let height = app.simulator().time().block_height();
                    let block = make_cold_transfer_block(height, n, accounts.atom, &participants);
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

criterion_group!(
    benches,
    bench_apply_block,
    bench_apply_block_with_trace,
    bench_apply_block_hot_cold
);
criterion_main!(benches);
