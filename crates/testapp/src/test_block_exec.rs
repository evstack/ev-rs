use crate::{account_codes, do_genesis, Block, TestAppStf};
use evolve_server_core::WritableKV;
use std::collections::HashMap;

#[test]
fn test_block_exec() {
    let mut codes = account_codes();
    let mut storage: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

    // do genesis
    do_genesis(&mut storage, &mut codes).unwrap();

    // execute first block
    let block = Block {
        height: 0,
        txs: vec![],
    };

    let (block) = TestAppStf::apply_block(&mut storage, &mut codes, &block);
    storage.apply_changes(block.state_changes).unwrap();
}
