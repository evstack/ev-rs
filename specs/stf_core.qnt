// Evolve SDK - State Transition Function (STF) Specification
//
// This Quint spec models the core STF logic:
//   - Block lifecycle: begin_block -> process txs -> end_block
//   - Transaction lifecycle: bootstrap -> validate -> execute -> post_tx
//   - Gas metering with per-tx limits and block gas limits
//   - Storage overlay with checkpoint/restore for atomicity
//   - Determinism: same block on same state => same result
//
// The model abstracts away concrete account code and message formats,
// focusing on the STF orchestration and invariants.

module stf {

  //
  // === Types ===
  //

  type AccountId = int
  type Key = List[int]
  type Value = List[int]

  /// Result of an operation: either Ok or an error code.
  type Result = { ok: bool, err_code: int }

  val OK: Result = { ok: true, err_code: 0 }
  pure def Err(code: int): Result = { ok: false, err_code: code }

  // Error codes (matching Rust implementation)
  val ERR_OUT_OF_GAS:     int = 0x01
  val ERR_CALL_DEPTH:     int = 0x02
  val ERR_VALIDATION:     int = 100
  val ERR_EXECUTION:      int = 200
  val ERR_POST_TX:        int = 999
  val ERR_BOOTSTRAP:      int = 300

  /// Storage gas configuration: cost per byte for get/set/remove.
  type GasConfig = {
    get_charge:    int,
    set_charge:    int,
    remove_charge: int,
  }

  /// A transaction to be executed.
  type Tx = {
    sender:           AccountId,
    recipient:        AccountId,
    gas_limit:        int,
    // Abstract payload: key/value the tx wants to write
    write_key:        Key,
    write_value:      Value,
    // Control flags for modeling different outcomes
    fail_validate:    bool,
    fail_execute:     bool,
    needs_bootstrap:  bool,
    fail_bootstrap:   bool,
  }

  /// A block containing transactions and context.
  type Block = {
    height:    int,
    time:      int,
    txs:       List[Tx],
    gas_limit: int,
  }

  /// Result of a single transaction execution.
  type TxResult = {
    result:   Result,
    gas_used: int,
  }

  /// Result of block execution.
  type BlockResult = {
    tx_results:  List[TxResult],
    gas_used:    int,
    txs_skipped: int,
  }

  //
  // === State ===
  //

  /// The persistent key-value store. Maps (account_id, key) -> value.
  var storage: AccountId -> (Key -> Value)

  /// Set of registered account IDs (have account code assigned).
  var accounts: Set[AccountId]

  /// Current block height (monotonically increasing).
  var block_height: int

  /// The last block result produced.
  var last_result: BlockResult

  /// Ghost: number of txs submitted in the last applied block.
  var last_block_tx_count: int

  /// Gas configuration (fixed at construction).
  val gas_config: GasConfig = { get_charge: 1, set_charge: 1, remove_charge: 1 }

  //
  // === Constants for model checking ===
  //

  val MC_ACCOUNTS: Set[AccountId] = Set(100, 101, 200, 201)
  val MC_GAS_LIMITS: Set[int] = Set(1, 40, 100, 10000)
  val MC_BLOCK_GAS_LIMITS: Set[int] = Set(30, 70, 1000, 1000000)

  //
  // === Pure helpers ===
  //

  /// AccountId is canonical 32-byte storage in the Rust implementation.
  /// The real storage key is `account_id_bytes ++ user_key`.
  val ACCOUNT_ID_BYTE_SIZE: int = 32
  val ACCOUNT_CODE_PREFIX_SIZE: int = 1
  val BORSH_LEN_PREFIX_SIZE: int = 4
  val TEST_ACCOUNT_CODE_ID_LEN: int = 12 // "test_account"

  /// Compute gas cost for a storage write operation.
  /// Real formula: set_charge * (full_key_len + 1 + value_len + 1)
  /// where full_key_len = ACCOUNT_ID_BYTE_SIZE + user_key.length()
  pure def write_gas_cost(config: GasConfig, key: Key, value: Value): int =
    config.set_charge * (ACCOUNT_ID_BYTE_SIZE + key.length() + 1 + value.length() + 1)

  /// Gas cost for sender bootstrap account-code registration.
  /// Real runtime charges storage set gas for:
  /// key   = ACCOUNT_IDENTIFIER_PREFIX (1 byte) ++ account_id (32 bytes)
  /// value = Borsh-encoded code id string ("test_account" => 4-byte len + 12 bytes)
  pure def bootstrap_gas_cost(config: GasConfig): int =
    config.set_charge * (
      ACCOUNT_CODE_PREFIX_SIZE + ACCOUNT_ID_BYTE_SIZE + 1 +
      BORSH_LEN_PREFIX_SIZE + TEST_ACCOUNT_CODE_ID_LEN + 1
    )

  /// Determine whether a transaction's write would exceed its gas limit.
  pure def would_exceed_gas(config: GasConfig, tx: Tx): bool =
    write_gas_cost(config, tx.write_key, tx.write_value) > tx.gas_limit

  /// Process a single transaction against the current state.
  /// Returns (tx_result, updated_account_storage, updated_accounts).
  ///
  /// Models the full tx lifecycle:
  ///   1. Optional sender bootstrap (account registration)
  ///   2. Sender existence check
  ///   3. Validation
  ///   4. Recipient existence check
  ///   5. Execution (write to storage)
  ///   6. Rollback on any failure
  pure def process_tx(
    config: GasConfig,
    tx: Tx,
    account_store: AccountId -> (Key -> Value),
    registered: Set[AccountId],
  ): { result: TxResult, store: AccountId -> (Key -> Value), accounts: Set[AccountId] } =
    // Phase 0: Bootstrap sender if needed
    val bootstrap_gas = bootstrap_gas_cost(config)
    val bootstrap_result =
      if (tx.needs_bootstrap and not(registered.contains(tx.sender)))
        if (bootstrap_gas > tx.gas_limit)
          // Bootstrap consumes storage gas first; if that exceeds the tx budget,
          // the runtime returns out-of-gas before running account init.
          { ok: false, err_code: ERR_OUT_OF_GAS, store: account_store, accounts: registered, gas: 0 }
        else if (tx.fail_bootstrap)
          // The runtime persists account registration before init runs, so an
          // init failure still leaves the sender registered with empty storage.
          {
            ok: false,
            err_code: ERR_BOOTSTRAP,
            store: account_store.put(tx.sender, Map()),
            accounts: registered.union(Set(tx.sender)),
            gas: bootstrap_gas,
          }
        else
          // Bootstrap succeeds: register sender and expose an empty user-store.
          { ok: true, err_code: 0, store: account_store.put(tx.sender, Map()), accounts: registered.union(Set(tx.sender)), gas: bootstrap_gas }
      else
        // No bootstrap needed or already registered
        { ok: true, err_code: 0, store: account_store, accounts: registered, gas: 0 }

    if (not(bootstrap_result.ok))
      // Bootstrap failed -> return the runtime-visible post-bootstrap state.
      {
        result: { result: Err(bootstrap_result.err_code), gas_used: bootstrap_result.gas },
        store: bootstrap_result.store,
        accounts: bootstrap_result.accounts,
      }
    else
      val current_accounts = bootstrap_result.accounts
      val current_store = bootstrap_result.store
      val gas_after_bootstrap = bootstrap_result.gas

      // Phase 1: Validation
      // In the real runtime the pluggable TxValidator verifies signatures by
      // calling into the sender's account code.  If the sender is not
      // registered (no account code), validation fails.
      if (tx.fail_validate or not(current_accounts.contains(tx.sender)))
        {
          result: { result: Err(ERR_VALIDATION), gas_used: gas_after_bootstrap },
          store: current_store,
          accounts: current_accounts,
        }
      else
        // Phase 2: Execution - compute gas for the write
        val gas_needed = write_gas_cost(config, tx.write_key, tx.write_value)
        val total_gas = gas_after_bootstrap + gas_needed

        if (total_gas > tx.gas_limit)
          // Out of gas: the real GasCounter does NOT increment gas_used on
          // a failed consume_gas call. So gas_used stays at its pre-call value.
          {
            result: { result: Err(ERR_OUT_OF_GAS), gas_used: gas_after_bootstrap },
            store: current_store,
            accounts: current_accounts,
          }
        else if (tx.fail_execute)
          // Execution error after write -> rollback
          {
            result: { result: Err(ERR_EXECUTION), gas_used: total_gas },
            store: current_store,
            accounts: current_accounts,
          }
        else
          // Phase 3: Write succeeds -> apply to storage
          val recipient_store =
            if (current_store.keys().contains(tx.recipient))
              current_store.get(tx.recipient)
            else
              Map()
          val updated_recipient = recipient_store.put(tx.write_key, tx.write_value)
          val updated_store = current_store.put(tx.recipient, updated_recipient)
          {
            result: { result: OK, gas_used: total_gas },
            store: updated_store,
            accounts: current_accounts,
          }

  /// Process all transactions in a block sequentially, enforcing block gas limit.
  pure def process_block(
    config: GasConfig,
    block: Block,
    account_store: AccountId -> (Key -> Value),
    registered: Set[AccountId],
  ): { result: BlockResult, store: AccountId -> (Key -> Value), accounts: Set[AccountId] } =
    val init_state = {
      tx_results: List(),
      cumulative_gas: 0,
      txs_skipped: 0,
      store: account_store,
      accounts: registered,
      stopped: false,
    }
    val final_state = block.txs.foldl(init_state, (acc, tx) =>
      if (acc.stopped)
        // Already hit block gas limit, skip remaining
        acc.with("txs_skipped", acc.txs_skipped + 1)
      else
        // Check if this tx would exceed block gas limit (pre-execution check)
        if (acc.cumulative_gas + tx.gas_limit > block.gas_limit)
          acc.with("txs_skipped", acc.txs_skipped + 1)
        else
          val tx_out = process_tx(config, tx, acc.store, acc.accounts)
          val new_cumulative = acc.cumulative_gas + tx_out.result.gas_used
          val should_stop = new_cumulative >= block.gas_limit
          {
            tx_results:     acc.tx_results.append(tx_out.result),
            cumulative_gas: new_cumulative,
            txs_skipped:    acc.txs_skipped,
            store:          tx_out.store,
            accounts:       tx_out.accounts,
            stopped:        should_stop,
          }
    )
    {
      result: {
        tx_results:  final_state.tx_results,
        gas_used:    final_state.cumulative_gas,
        txs_skipped: final_state.txs_skipped,
      },
      store:    final_state.store,
      accounts: final_state.accounts,
    }

  //
  // === Actions (state transitions) ===
  //

  action init = all {
    storage'              = Map(),
    accounts'             = Set(),
    block_height'         = 0,
    last_result'          = { tx_results: List(), gas_used: 0, txs_skipped: 0 },
    last_block_tx_count'  = 0,
  }

  /// Apply a block to the current state.
  action apply_block(block: Block): bool =
    val out = process_block(gas_config, block, storage, accounts)
    all {
      // Block height must be monotonically increasing.
      block.height > block_height,
      block.gas_limit > 0,
      storage'              = out.store,
      accounts'             = out.accounts,
      block_height'         = block.height,
      last_result'          = out.result,
      last_block_tx_count'  = block.txs.length(),
    }

  /// Driver-friendly block wrapper for `quint_connect`.
  /// The STF can only execute against accounts with registered code, so this
  /// wrapper aligns the model with the real runtime by requiring a registered recipient.
  action apply_registered_block(
    sender: AccountId,
    recipient: AccountId,
    gas_limit: int,
    block_gas: int,
    fail_v: bool,
    fail_e: bool,
    needs_b: bool,
    fail_b: bool,
  ): bool =
    val tx: Tx = {
      sender: sender,
      recipient: recipient,
      gas_limit: gas_limit,
      write_key: K1,
      write_value: V1,
      fail_validate: fail_v,
      fail_execute: fail_e,
      needs_bootstrap: needs_b,
      fail_bootstrap: fail_b,
    }
    all {
      accounts.contains(recipient),
      apply_block({
        height: block_height + 1,
        time: 0,
        txs: [tx],
        gas_limit: block_gas,
      }),
    }

  /// Register an account (e.g., during genesis).
  action register_account(id: AccountId): bool = all {
    not(accounts.contains(id)),
    storage'              = storage.put(id, Map()),
    accounts'             = accounts.union(Set(id)),
    block_height'         = block_height,
    last_result'          = last_result,
    last_block_tx_count'  = last_block_tx_count,
  }

  //
  // === Invariants ===
  //

  /// INV-1: Block height is non-negative.
  /// Monotonicity is enforced structurally by the guard block.height > block_height
  /// in apply_block. This invariant checks the weaker non-negativity property.
  val height_non_negative: bool = block_height >= 0

  /// INV-2: Total gas reported equals sum of individual tx gas.
  val gas_accounting: bool =
    val sum = last_result.tx_results.foldl(0, (acc, tr) => acc + tr.gas_used)
    last_result.gas_used == sum

  /// INV-3: Number of results + skipped = number of txs submitted.
  val result_completeness: bool =
    last_result.tx_results.length() + last_result.txs_skipped == last_block_tx_count

  /// INV-4: Failed transactions do not modify storage.
  /// (Encoded structurally: process_tx returns the original store on failure.)

  /// INV-5: Only registered accounts appear as storage keys.
  val storage_accounts_registered: bool =
    storage.keys().forall(id => accounts.contains(id))

  /// INV-6: Gas used is always non-negative.
  val gas_non_negative: bool =
    last_result.tx_results.foldl(true, (acc, tr) => acc and tr.gas_used >= 0)

  //
  // === False-invariant witnesses (should be violated) ===
  //

  /// Should be violated: proves blocks with txs are processed.
  val witnessNoTxsProcessed: bool = last_result.tx_results.length() == 0

  /// Should be violated: proves successful txs can write to storage.
  val witnessStorageNeverWritten: bool =
    storage.keys().forall(id => storage.get(id) == Map())

  /// Should be violated: proves block gas limit causes skips.
  val witnessNoSkippedTxs: bool = last_result.txs_skipped == 0

  /// Should be violated: proves bootstrap can register new accounts.
  val witnessNoBootstrap: bool = accounts.size() <= 1

  //
  // === Temporal Properties ===
  //

  /// PROP-1: Determinism - same block on same state produces identical results.
  /// This is ensured structurally: all functions are pure, no randomness,
  /// no non-deterministic collection iteration (BTreeMap in Rust, ordered Map in Quint).

  /// PROP-2: If a transaction has fail_validate=true, it always returns ERR_VALIDATION.
  /// PROP-3: If a transaction runs out of gas, it always returns ERR_OUT_OF_GAS.
  /// PROP-4: Successful transactions always produce storage changes.
  /// These are verified by the process_tx function structure.

  //
  // === Nondeterministic step for model checking ===
  //

  action step = {
    nondet id = MC_ACCOUNTS.oneOf()
    nondet sender = MC_ACCOUNTS.oneOf()
    nondet recipient = MC_ACCOUNTS.oneOf()
    nondet gas_limit = MC_GAS_LIMITS.oneOf()
    nondet block_gas = MC_BLOCK_GAS_LIMITS.oneOf()
    nondet fail_v = Set(true, false).oneOf()
    nondet fail_e = Set(true, false).oneOf()
    nondet needs_b = Set(true, false).oneOf()
    nondet fail_b = Set(true, false).oneOf()
    val tx: Tx = {
      sender: sender, recipient: recipient, gas_limit: gas_limit,
      write_key: K1, write_value: V1,
      fail_validate: fail_v, fail_execute: fail_e,
      needs_bootstrap: needs_b, fail_bootstrap: fail_b,
    }
    any {
      apply_block({
        height: block_height + 1, time: 0,
        txs: [tx], gas_limit: block_gas,
      }),
      register_account(id),
    }
  }

  /// Alternate MBT step for `quint_connect`: keeps the same exploration
  /// parameters, but only dispatches to runtime-valid block applications.
  action qc_step = {
    nondet id = MC_ACCOUNTS.oneOf()
    nondet sender = MC_ACCOUNTS.oneOf()
    nondet recipient = MC_ACCOUNTS.oneOf()
    nondet gas_limit = MC_GAS_LIMITS.oneOf()
    nondet block_gas = MC_BLOCK_GAS_LIMITS.oneOf()
    nondet fail_v = Set(true, false).oneOf()
    nondet fail_e = Set(true, false).oneOf()
    nondet needs_b = Set(true, false).oneOf()
    nondet fail_b = Set(true, false).oneOf()
    any {
      apply_registered_block(sender, recipient, gas_limit, block_gas, fail_v, fail_e, needs_b, fail_b),
      register_account(id),
    }
  }

  //
  // === Test helpers ===
  //

  // Byte-list keys/values for test readability
  val K0: Key = [0]
  val K1: Key = [1]
  val K2: Key = [2]
  val K3: Key = [3]
  val V0: Value = [10]
  val V1: Value = [11]
  val V2: Value = [12]
  val V3: Value = [13]
  val V_OLD: Value = [20]
  val V_NEW: Value = [21]

  /// Stutter step: keep all state unchanged (required to end a run).
  action stutter: bool = all {
    storage' = storage,
    accounts' = accounts,
    block_height' = block_height,
    last_result' = last_result,
    last_block_tx_count' = last_block_tx_count,
  }

  //
  // === Tests ===
  //

  /// Test: empty block produces no results and no state changes.
  run emptyBlockTest = {
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: List(), gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results.length() == 0),
        assert(last_result.gas_used == 0),
        assert(last_result.txs_skipped == 0),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: successful transaction writes to storage.
  run successfulTxTest = {
    val tx1: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results.length() == 1),
        assert(last_result.tx_results[0].result.ok == true),
        assert(storage.get(100).get(K1) == V1),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: validation failure does not modify storage.
  run validationFailureTest = {
    val tx1: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: true, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results[0].result.ok == false),
        assert(last_result.tx_results[0].result.err_code == ERR_VALIDATION),
        assert(storage.get(100) == Map()),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: execution failure rolls back storage write.
  run executionFailureRollbackTest = {
    val tx1: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: true,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results[0].result.ok == false),
        assert(last_result.tx_results[0].result.err_code == ERR_EXECUTION),
        assert(storage.get(100) == Map()),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: out-of-gas transaction fails and does not write.
  run outOfGasTest = {
    val tx1: Tx = {
      sender: 100, recipient: 100, gas_limit: 1,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results[0].result.ok == false),
        assert(last_result.tx_results[0].result.err_code == ERR_OUT_OF_GAS),
        assert(storage.get(100) == Map()),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: block gas limit causes transactions to be skipped.
  /// With Key=[1], Value=[11]: gas_cost = 1*(32+1+1+1+1) = 36 per tx.
  /// tx1.gas_limit=40, tx2.gas_limit=40. Block gas_limit=70.
  /// Pre-check: cumulative(0) + tx1.gas_limit(40) = 40 <= 70 -> execute tx1 (uses 36 gas).
  /// Pre-check: cumulative(36) + tx2.gas_limit(40) = 76 > 70 -> skip tx2.
  run blockGasLimitTest = {
    val tx1: Tx = {
      sender: 100, recipient: 100, gas_limit: 40,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    val tx2: Tx = {
      sender: 100, recipient: 100, gas_limit: 40,
      write_key: K2, write_value: V2,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1, tx2], gas_limit: 70 }))
      .then(all {
        assert(last_result.tx_results.length() == 1),
        assert(last_result.txs_skipped == 1),
        assert(last_result.tx_results[0].result.ok == true),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: mixed block with success, validation failure, execution failure, OOG.
  run mixedOutcomesTest = {
    val tx_ok: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K0, write_value: V0,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    val tx_validate_fail: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: true, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    val tx_exec_fail: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K2, write_value: V2,
      fail_validate: false, fail_execute: true,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    val tx_oog: Tx = {
      sender: 100, recipient: 100, gas_limit: 1,
      write_key: K3, write_value: V3,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({
        height: 1, time: 0,
        txs: [tx_ok, tx_validate_fail, tx_exec_fail, tx_oog],
        gas_limit: 1000000,
      }))
      .then(all {
        assert(last_result.tx_results.length() == 4),
        // tx_ok succeeds
        assert(last_result.tx_results[0].result.ok == true),
        // tx_validate_fail fails validation
        assert(last_result.tx_results[1].result.err_code == ERR_VALIDATION),
        // tx_exec_fail fails execution
        assert(last_result.tx_results[2].result.err_code == ERR_EXECUTION),
        // tx_oog fails with out of gas
        assert(last_result.tx_results[3].result.err_code == ERR_OUT_OF_GAS),
        // Only the first tx's write should be in storage
        assert(storage.get(100).get(K0) == V0),
        assert(not(storage.get(100).keys().contains(K1))),
        assert(not(storage.get(100).keys().contains(K2))),
        assert(not(storage.get(100).keys().contains(K3))),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: sender bootstrap registers account before execution.
  run bootstrapTest = {
    val tx1: Tx = {
      sender: 200, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: true, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results[0].result.ok == true),
        // Sender should now be registered
        assert(accounts.contains(200)),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: failed bootstrap prevents execution.
  run bootstrapFailureTest = {
    val tx1: Tx = {
      sender: 200, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: true, fail_bootstrap: true,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results[0].result.ok == false),
        assert(last_result.tx_results[0].result.err_code == ERR_BOOTSTRAP),
        // Registration persists even though init failed.
        assert(accounts.contains(200)),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: unregistered sender (no bootstrap) fails validation.
  /// In production, the validator queries the sender's account code for
  /// signature verification.  Without account code the query fails.
  run unregisteredSenderTest = {
    val tx1: Tx = {
      sender: 200, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(all {
        assert(last_result.tx_results[0].result.ok == false),
        assert(last_result.tx_results[0].result.err_code == ERR_VALIDATION),
        assert(storage.get(100) == Map()),
        assert(result_completeness),
        stutter,
      })
  }

  /// Test: sequential blocks accumulate state correctly.
  run multiBlockTest = {
    val tx1: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V1,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    val tx2: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K2, write_value: V2,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1], gas_limit: 1000000 }))
      .then(apply_block({ height: 2, time: 10, txs: [tx2], gas_limit: 1000000 }))
      .then(all {
        // Both writes should be present
        assert(storage.get(100).get(K1) == V1),
        assert(storage.get(100).get(K2) == V2),
        assert(block_height == 2),
        stutter,
      })
  }

  /// Test: overwriting a key updates the value.
  run overwriteTest = {
    val tx1: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V_OLD,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    val tx2: Tx = {
      sender: 100, recipient: 100, gas_limit: 10000,
      write_key: K1, write_value: V_NEW,
      fail_validate: false, fail_execute: false,
      needs_bootstrap: false, fail_bootstrap: false,
    }
    init
      .then(register_account(100))
      .then(apply_block({ height: 1, time: 0, txs: [tx1, tx2], gas_limit: 1000000 }))
      .then(all {
        assert(storage.get(100).get(K1) == V_NEW),
        stutter,
      })
  }
}
