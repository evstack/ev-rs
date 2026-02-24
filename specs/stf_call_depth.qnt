// Evolve SDK - STF Call Depth Specification
//
// Focus: call depth enforcement for nested do_exec paths.

module stf_call_depth {
  type Result = { ok: bool, err_code: int }
  val OK: Result = { ok: true, err_code: 0 }
  pure def Err(code: int): Result = { ok: false, err_code: code }

  val ERR_CALL_DEPTH: int = 0x14
  val MAX_CALL_DEPTH: int = 64

  type Tx = {
    requested_depth: int,
  }

  type TxResult = {
    result: Result,
  }

  var last_result: TxResult

  pure def process_tx(tx: Tx): TxResult =
    // Runtime rejects when call_depth >= MAX_CALL_DEPTH before the next call.
    if (tx.requested_depth >= MAX_CALL_DEPTH)
      { result: Err(ERR_CALL_DEPTH) }
    else
      { result: OK }

  action init = all {
    last_result' = { result: OK },
  }

  action apply_tx(tx: Tx): bool = all {
    last_result' = process_tx(tx),
  }

  action stutter: bool = all {
    last_result' = last_result,
  }

  run callDepthBelowLimitSucceedsTest = {
    init
      .then(apply_tx({ requested_depth: 63 }))
      .then(all {
        assert(last_result.result.ok == true),
        stutter,
      })
  }

  run callDepthAtLimitFailsTest = {
    init
      .then(apply_tx({ requested_depth: 64 }))
      .then(all {
        assert(last_result.result.ok == false),
        assert(last_result.result.err_code == ERR_CALL_DEPTH),
        stutter,
      })
  }
}
