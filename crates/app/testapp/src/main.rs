use evolve_testapp::{do_genesis, install_account_codes, STF};
use evolve_testing::server_mocks::AccountStorageMock;

fn main() {
    let storage = evolve_testapp::storage::Storage::default();

    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    // Run genesis
    let state = do_genesis(&STF, &codes, &storage).expect("genesis failed");
    let _changes = state.into_changes().expect("failed to get state changes");

    println!("Evolve testapp initialized successfully");
    println!("To run a full node, integrate with a consensus engine of your choice.");
}
