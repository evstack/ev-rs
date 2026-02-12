use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use evolve_debugger::ExecutionTrace;
use evolve_simulator::SimConfig;
use evolve_testapp::sim_testing::SimTestApp;

fn make_transfer_txs(
    app: &mut SimTestApp,
    tx_count: usize,
    atom_id: evolve_core::AccountId,
    alice: evolve_core::AccountId,
    bob: evolve_core::AccountId,
) -> Vec<Vec<u8>> {
    let mut txs = Vec::with_capacity(tx_count);
    for i in 0..tx_count {
        let (sender, recipient) = if i % 2 == 0 {
            (alice, bob)
        } else {
            (bob, alice)
        };
        let raw = app
            .build_token_transfer_tx(sender, atom_id, recipient, 1, 100_000)
            .expect("build tx");
        txs.push(raw);
    }

    txs
}

fn make_transfer_tx_contexts(
    app: &mut SimTestApp,
    tx_count: usize,
    atom_id: evolve_core::AccountId,
    alice: evolve_core::AccountId,
    bob: evolve_core::AccountId,
) -> Vec<evolve_tx_eth::TxContext> {
    let mut txs = Vec::with_capacity(tx_count);
    for i in 0..tx_count {
        let (sender, recipient) = if i % 2 == 0 {
            (alice, bob)
        } else {
            (bob, alice)
        };
        let tx = app
            .build_token_transfer_tx_context(sender, atom_id, recipient, 1, 100_000)
            .expect("build tx context");
        txs.push(tx);
    }
    txs
}

fn make_cold_transfer_txs(
    app: &mut SimTestApp,
    tx_count: usize,
    atom_id: evolve_core::AccountId,
    accounts: &[evolve_core::AccountId],
) -> Vec<Vec<u8>> {
    let mut txs = Vec::with_capacity(tx_count);
    let count = accounts.len();
    for i in 0..tx_count {
        let sender = accounts[i % count];
        let recipient = accounts[(i + 1) % count];
        let raw = app
            .build_token_transfer_tx(sender, atom_id, recipient, 1, 100_000)
            .expect("build tx");
        txs.push(raw);
    }

    txs
}

fn bench_apply_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("stf_apply_block");
    for tx_count in [10usize, 100, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(tx_count), &tx_count, |b, &n| {
            b.iter_batched(
                || {
                    let mut app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let txs =
                        make_transfer_txs(&mut app, n, accounts.atom, accounts.alice, accounts.bob);
                    (app, txs)
                },
                |(mut app, txs)| {
                    for raw_tx in &txs {
                        app.submit_raw_tx(raw_tx).expect("submit tx");
                    }
                    app.produce_block_from_mempool(n);
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
                    let mut app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let txs =
                        make_transfer_txs(&mut app, n, accounts.atom, accounts.alice, accounts.bob);
                    (app, txs)
                },
                |(mut app, txs)| {
                    let (_results, _trace): (Vec<_>, ExecutionTrace) =
                        app.run_blocks_with_trace(1, |_height, _app| txs.clone());
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
                    let mut app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let txs =
                        make_transfer_txs(&mut app, n, accounts.atom, accounts.alice, accounts.bob);
                    (app, txs)
                },
                |(mut app, txs)| {
                    for raw_tx in &txs {
                        app.submit_raw_tx(raw_tx).expect("submit tx");
                    }
                    app.produce_block_from_mempool(n);
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
                    let txs = make_cold_transfer_txs(&mut app, n, accounts.atom, &participants);
                    (app, txs)
                },
                |(mut app, txs)| {
                    for raw_tx in &txs {
                        app.submit_raw_tx(raw_tx).expect("submit tx");
                    }
                    app.produce_block_from_mempool(n);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_stf_only_apply_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("stf_only_apply_block");
    for tx_count in [100usize, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(tx_count), &tx_count, |b, &n| {
            b.iter_batched(
                || {
                    let mut app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let txs = make_transfer_tx_contexts(
                        &mut app,
                        n,
                        accounts.atom,
                        accounts.alice,
                        accounts.bob,
                    );
                    (app, txs)
                },
                |(mut app, txs)| {
                    app.produce_block_with_txs(txs.clone());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_mempool_stf_apply_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("mempool_stf_apply_block");
    for tx_count in [100usize, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(tx_count), &tx_count, |b, &n| {
            b.iter_batched(
                || {
                    let mut app = SimTestApp::with_config(SimConfig::replay(), 42);
                    let accounts = app.accounts();
                    let txs = make_transfer_tx_contexts(
                        &mut app,
                        n,
                        accounts.atom,
                        accounts.alice,
                        accounts.bob,
                    );
                    (app, txs)
                },
                |(mut app, txs)| {
                    for tx in txs {
                        app.submit_tx_context(tx).expect("submit tx context");
                    }
                    app.produce_block_from_mempool(n);
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
    bench_apply_block_hot_cold,
    bench_stf_only_apply_block,
    bench_mempool_stf_apply_block
);
criterion_main!(benches);
