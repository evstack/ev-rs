module TokenV1

// Canonical specgen directives for token_v1 code generation.
// @specgen module_kind=token_v1
// @specgen rust_mod=generated_token
// @specgen account_type=TokenGenerated
// @specgen error.not_enough_balance.code=0x1
// @specgen error.not_enough_balance.message=not enough balance
// @specgen error.underflow.code=0x2
// @specgen error.underflow.message=arithmetic underflow
// @specgen slot.metadata=0
// @specgen slot.balances=1
// @specgen slot.total_supply=2
// @specgen slot.supply_manager=3

// Quint subset declarations required by specgen.
action initialize(metadata, balances, supply_manager) = true
action mint(recipient, amount) = true
action burn(from_account, amount) = true
action transfer(to, amount) = true
action freeze(account) = true

def metadata(accountState) = accountState.metadata
def get_balance(account, accountState) = accountState.balances.get(account)
def total_supply(accountState) = accountState.total_supply
def is_frozen(account, accountState) = false
