// Evolve SDK - STF Post-Tx Hook Specification
//
// Focus: post-tx handler semantics.
// In the runtime, the post-tx handler runs AFTER the execution checkpoint
// is dropped. A post-tx error overrides the tx response, but state changes
// from execution remain committed.
//
// This spec verifies:
//   - Post-tx rejection keeps execution state changes
//   - Post-tx rejection does not mask execution failures
//   - Gas accounting remains correct across post-tx outcomes

module stf_post_tx {
  type AccountId = int
  type Key = List[int]
  type Value = List[int]

  type Result = { ok: bool, err_code: int }
  val OK: Result = { ok: true, err_code: 0 }
  pure def Err(code: int): Result = { ok: false, err_code: code }

  val ERR_OUT_OF_GAS: int = 0x01
  val ERR_EXECUTION: int = 200
  val ERR_POST_TX: int = 999

  type GasConfig = {
    set_charge: int,
  }

  type Tx = {
    recipient:       AccountId,
    gas_limit:       int,
    write_key:       Key,
    write_value:     Value,
    fail_execute:    bool,
    reject_post_tx:  bool,
  }

  type TxResult = {
    result:   Result,
    gas_used: int,
  }

  type BlockResult = {
    tx_results: List[TxResult],
    gas_used:   int,
  }

  //
  // === State ===
  //

  var storage: AccountId -> (Key -> Value)
  var accounts: Set[AccountId]
  var last_result: BlockResult

  /// Constants for model checking.
  val MC_ACCOUNTS: Set[AccountId] = Set(100, 101)

  val gas_config: GasConfig = { set_charge: 1 }
  val ACCOUNT_ID_BYTE_SIZE: int = 16

  pure def write_gas_cost(config: GasConfig, key: Key, value: Value): int =
    config.set_charge * (ACCOUNT_ID_BYTE_SIZE + key.length() + 1 + value.length() + 1)

  pure def process_tx(
    config: GasConfig,
    tx: Tx,
    account_store: AccountId -> (Key -> Value),
  ): { result: TxResult, store: AccountId -> (Key -> Value) } =
    val gas_needed = write_gas_cost(config, tx.write_key, tx.write_value)
    if (gas_needed > tx.gas_limit)
      {
        result: { result: Err(ERR_OUT_OF_GAS), gas_used: 0 },
        store: account_store,
      }
    else if (tx.fail_execute)
      // Runtime do_exec rolls back storage on execution error.
      // Post-tx does not run after execution failure.
      {
        result: { result: Err(ERR_EXECUTION), gas_used: gas_needed },
        store: account_store,
      }
    else
      // Execution succeeds and writes state.
      val recipient_store =
        if (account_store.keys().contains(tx.recipient))
          account_store.get(tx.recipient)
        else
          Map()
      val updated_recipient = recipient_store.put(tx.write_key, tx.write_value)
      val updated_store = account_store.put(tx.recipient, updated_recipient)
      // Post-tx handler runs AFTER checkpoint is dropped.
      // It can override the response but NOT roll back state.
      {
        result: {
          result: if (tx.reject_post_tx) Err(ERR_POST_TX) else OK,
          gas_used: gas_needed,
        },
        store: updated_store,
      }

  pure def process_block(
    config: GasConfig,
    txs: List[Tx],
    account_store: AccountId -> (Key -> Value),
  ): { result: BlockResult, store: AccountId -> (Key -> Value) } =
    val init_state = {
      tx_results: List(),
      cumulative_gas: 0,
      store: account_store,
    }
    val final_state = txs.foldl(init_state, (acc, tx) =>
      val tx_out = process_tx(config, tx, acc.store)
      {
        tx_results: acc.tx_results.append(tx_out.result),
        cumulative_gas: acc.cumulative_gas + tx_out.result.gas_used,
        store: tx_out.store,
      }
    )
    {
      result: { tx_results: final_state.tx_results, gas_used: final_state.cumulative_gas },
      store: final_state.store,
    }

  //
  // === Actions ===
  //

  action init = all {
    storage' = Map(),
    accounts' = Set(),
    last_result' = { tx_results: List(), gas_used: 0 },
  }

  action register_account(id: AccountId): bool = all {
    not(accounts.contains(id)),
    storage' = storage.put(id, Map()),
    accounts' = accounts.union(Set(id)),
    last_result' = last_result,
  }

  action apply_txs(txs: List[Tx]): bool =
    val out = process_block(gas_config, txs, storage)
    all {
      storage' = out.store,
      accounts' = accounts,
      last_result' = out.result,
    }

  action stutter: bool = all {
    storage' = storage,
    accounts' = accounts,
    last_result' = last_result,
  }

  //
  // === Nondeterministic step for model checking ===
  //

  action step = {
    nondet recipient = MC_ACCOUNTS.oneOf()
    nondet gas_limit = Set(1, 100, 10000).oneOf()
    nondet fail_e = Set(true, false).oneOf()
    nondet reject_pt = Set(true, false).oneOf()
    val tx: Tx = {
      recipient: recipient,
      gas_limit: gas_limit,
      write_key: K1,
      write_value: V1,
      fail_execute: fail_e,
      reject_post_tx: reject_pt,
    }
    any {
      apply_txs([tx]),
      register_account(recipient),
    }
  }

  //
  // === Invariants ===
  //

  /// INV-1: Gas accounting -- total equals sum of individual tx gas.
  val gas_accounting: bool =
    last_result.tx_results.foldl(0, (acc, tr) => acc + tr.gas_used) == last_result.gas_used

  /// INV-2: Gas is non-negative.
  val gas_non_negative: bool =
    last_result.tx_results.foldl(true, (acc, tr) => acc and tr.gas_used >= 0)

  /// INV-3: Only registered accounts have storage entries.
  val storage_accounts_registered: bool =
    storage.keys().forall(id => accounts.contains(id))

  //
  // === False-invariant witnesses (should be violated) ===
  //

  /// Should be violated: proves post-tx rejection path is reachable.
  val witnessNoPostTxReject: bool =
    last_result.tx_results.foldl(true, (acc, tr) =>
      acc and (tr.result.ok or tr.result.err_code != ERR_POST_TX))

  /// Should be violated: proves execution happens.
  val witnessNoExecution: bool = last_result.tx_results.length() == 0

  /// Should be violated: proves successful txs write storage.
  val witnessStorageEmpty: bool =
    storage.keys().forall(id => storage.get(id) == Map())

  //
  // === Test helpers ===
  //

  val K1: Key = [1]
  val V1: Value = [11]
  val K2: Key = [2]
  val V2: Value = [12]

  //
  // === Tests ===
  //

  /// Core property: post-tx rejection keeps execution state changes.
  /// This is the critical semantic: the handler runs after checkpoint drop.
  run postTxRejectsButKeepsStateTest = {
    val tx: Tx = {
      recipient: 100,
      gas_limit: 10000,
      write_key: K1,
      write_value: V1,
      fail_execute: false,
      reject_post_tx: true,
    }
    init
      .then(register_account(100))
      .then(apply_txs([tx]))
      .then(all {
        assert(last_result.tx_results.length() == 1),
        assert(last_result.tx_results[0].result.ok == false),
        assert(last_result.tx_results[0].result.err_code == ERR_POST_TX),
        // State change persists despite post-tx error
        assert(storage.get(100).get(K1) == V1),
        assert(gas_accounting),
        stutter,
      })
  }

  /// Post-tx does not mask execution failure (exec error takes precedence).
  run postTxDoesNotMaskExecFailureTest = {
    val tx: Tx = {
      recipient: 100,
      gas_limit: 10000,
      write_key: K1,
      write_value: V1,
      fail_execute: true,
      reject_post_tx: true,
    }
    init
      .then(register_account(100))
      .then(apply_txs([tx]))
      .then(all {
        assert(last_result.tx_results.length() == 1),
        assert(last_result.tx_results[0].result.ok == false),
        // Execution error, NOT post-tx error
        assert(last_result.tx_results[0].result.err_code == ERR_EXECUTION),
        // No state changes (execution was rolled back)
        assert(storage.get(100) == Map()),
        assert(gas_accounting),
        stutter,
      })
  }

  /// Happy path: no post-tx rejection, execution succeeds.
  run happyPathTest = {
    val tx: Tx = {
      recipient: 100,
      gas_limit: 10000,
      write_key: K1,
      write_value: V1,
      fail_execute: false,
      reject_post_tx: false,
    }
    init
      .then(register_account(100))
      .then(apply_txs([tx]))
      .then(all {
        assert(last_result.tx_results[0].result.ok == true),
        assert(storage.get(100).get(K1) == V1),
        assert(gas_accounting),
        stutter,
      })
  }

  /// Mixed block: one tx with post-tx rejection, one without.
  run mixedPostTxTest = {
    val tx_reject: Tx = {
      recipient: 100,
      gas_limit: 10000,
      write_key: K1,
      write_value: V1,
      fail_execute: false,
      reject_post_tx: true,
    }
    val tx_ok: Tx = {
      recipient: 100,
      gas_limit: 10000,
      write_key: K2,
      write_value: V2,
      fail_execute: false,
      reject_post_tx: false,
    }
    init
      .then(register_account(100))
      .then(apply_txs([tx_reject, tx_ok]))
      .then(all {
        assert(last_result.tx_results.length() == 2),
        // First tx: post-tx rejected, but state persists
        assert(last_result.tx_results[0].result.err_code == ERR_POST_TX),
        assert(storage.get(100).get(K1) == V1),
        // Second tx: fully succeeds
        assert(last_result.tx_results[1].result.ok == true),
        assert(storage.get(100).get(K2) == V2),
        assert(gas_accounting),
        stutter,
      })
  }

  /// Out-of-gas: post-tx flag is irrelevant since execution never completes.
  run outOfGasIgnoresPostTxTest = {
    val tx: Tx = {
      recipient: 100,
      gas_limit: 1,
      write_key: K1,
      write_value: V1,
      fail_execute: false,
      reject_post_tx: true,
    }
    init
      .then(register_account(100))
      .then(apply_txs([tx]))
      .then(all {
        assert(last_result.tx_results[0].result.err_code == ERR_OUT_OF_GAS),
        assert(storage.get(100) == Map()),
        assert(gas_accounting),
        stutter,
      })
  }
}
