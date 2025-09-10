use evolve_stf::SystemAccounts;
use evolve_testapp::{build_stf, do_genesis, install_account_codes, PLACEHOLDER_ACCOUNT};
use evolve_testing::server_mocks::AccountStorageMock;

fn main() {
    let storage = evolve_testapp::storage::Storage::default();

    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    // Run genesis
    let bootstrap_stf = build_stf(SystemAccounts::placeholder(), PLACEHOLDER_ACCOUNT);
    let (state, _accounts) = do_genesis(&bootstrap_stf, &codes, &storage).expect("genesis failed");
    let _changes = state.into_changes().expect("failed to get state changes");

    println!("Evolve testapp initialized successfully");
    println!("To run a full node, integrate with a consensus engine of your choice.");
}
