// Evolve SDK - STF Call Depth Specification
//
// Models nested do_exec calls with call depth accumulation.
// In the runtime, each do_exec/branch_exec increments call_depth
// via saturating_add. Calls are rejected when depth >= MAX_CALL_DEPTH.
//
// This spec models:
//   - Call stack growth through nested execution
//   - Depth enforcement at MAX_CALL_DEPTH (64)
//   - Stack unwinding on return
//   - Invariant: depth never exceeds the limit

module stf_call_depth {
  type AccountId = int
  type Result = { ok: bool, err_code: int }

  val OK: Result = { ok: true, err_code: 0 }
  pure def Err(code: int): Result = { ok: false, err_code: code }

  val ERR_CALL_DEPTH: int = 0x14
  val MAX_CALL_DEPTH: int = 64

  /// The set of account IDs for nondeterministic exploration.
  val ACCOUNTS: Set[AccountId] = Set(1, 2, 3)

  //
  // === State ===
  //

  /// Call stack: tracks the chain of nested calls.
  /// Length of this list is the current call depth.
  var call_stack: List[AccountId]

  /// Result of the last attempted operation.
  var last_result: Result

  //
  // === Actions ===
  //

  action init = all {
    call_stack' = List(),
    last_result' = OK,
  }

  /// Nested call. In the runtime this is do_exec -> branch_exec
  /// which increments call_depth. Succeeds only if depth < MAX.
  action do_exec(target: AccountId): bool = all {
    // Guard: depth check happens BEFORE checkpoint is taken
    call_stack.length() < MAX_CALL_DEPTH,
    // Update: push target onto call stack (depth increments)
    call_stack' = call_stack.append(target),
    last_result' = OK,
  }

  /// Call rejected because depth limit is reached.
  action do_exec_rejected(target: AccountId): bool = all {
    // Guard: at or above the limit
    call_stack.length() >= MAX_CALL_DEPTH,
    // No state change: rejected before any checkpoint
    call_stack' = call_stack,
    last_result' = Err(ERR_CALL_DEPTH),
  }

  /// Return from a call (pop the stack).
  action return_from_call: bool = all {
    // Guard: must have an active call
    call_stack.length() > 0,
    // Update: pop the last call
    call_stack' = call_stack.slice(0, call_stack.length() - 1),
    last_result' = OK,
  }

  /// Stutter step: no state change.
  action stutter: bool = all {
    call_stack' = call_stack,
    last_result' = last_result,
  }

  //
  // === Nondeterministic step for model checking ===
  //

  action step = {
    nondet target = ACCOUNTS.oneOf()
    any {
      do_exec(target),
      do_exec_rejected(target),
      return_from_call,
    }
  }

  //
  // === Invariants ===
  //

  /// INV-1: Call stack depth never exceeds MAX_CALL_DEPTH.
  val depthBounded: bool = call_stack.length() <= MAX_CALL_DEPTH

  /// INV-2: A rejected call always produces ERR_CALL_DEPTH.
  val rejectionCorrect: bool =
    not(last_result.ok) implies last_result.err_code == ERR_CALL_DEPTH

  /// INV-3: Call depth is non-negative (structural with List).
  val depthNonNegative: bool = call_stack.length() >= 0

  //
  // === False-invariant witnesses (should be violated) ===
  //

  /// Should be violated: proves calls can happen.
  val witnessNoCalls: bool = call_stack.length() == 0

  /// Should be violated: proves nested calls are reachable.
  val witnessAlwaysShallow: bool = call_stack.length() < 3

  /// Should be violated: proves call rejection is reachable.
  val witnessNoRejections: bool = last_result.ok

  //
  // === Tests ===
  //

  /// Test: single call succeeds and increments depth.
  run singleCallTest =
    init
      .then(do_exec(1))
      .then(all {
        assert(call_stack.length() == 1),
        assert(last_result.ok == true),
        assert(depthBounded),
        stutter,
      })

  /// Test: nested calls accumulate depth correctly.
  run nestedCallsTest =
    init
      .then(do_exec(1))
      .then(do_exec(2))
      .then(do_exec(3))
      .then(all {
        assert(call_stack.length() == 3),
        assert(call_stack[0] == 1),
        assert(call_stack[1] == 2),
        assert(call_stack[2] == 3),
        assert(depthBounded),
        stutter,
      })

  /// Test: return_from_call decrements depth.
  run returnUnwindsStackTest =
    init
      .then(do_exec(1))
      .then(do_exec(2))
      .then(return_from_call)
      .then(all {
        assert(call_stack.length() == 1),
        assert(call_stack[0] == 1),
        stutter,
      })

  /// Test: full unwind returns to empty stack.
  run fullUnwindTest =
    init
      .then(do_exec(1))
      .then(do_exec(2))
      .then(return_from_call)
      .then(return_from_call)
      .then(all {
        assert(call_stack.length() == 0),
        stutter,
      })

  /// Test: cannot return from empty stack.
  run cannotReturnFromEmptyTest =
    init
      .then(return_from_call)
      .fail()

  /// Test: recursive calls (same account) track depth.
  run recursiveCallsTest =
    init
      .then(do_exec(1))
      .then(do_exec(1))
      .then(do_exec(1))
      .then(all {
        assert(call_stack.length() == 3),
        assert(call_stack[0] == 1),
        assert(call_stack[1] == 1),
        assert(call_stack[2] == 1),
        stutter,
      })
}
