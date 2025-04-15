# Application Actor System Documentation

## Overview

The Application Actor system is a component of the CoTend/Evolve runtime that handles the execution of transactions,
state management, and the actor model implementation. This document explains the architecture, components, and flow of
the system.

## Core Components

### State Transition Function (STF)

The STF is the central component that orchestrates the execution of transactions and manages the state of the
blockchain. It provides functions for:

- Applying blocks (`apply_block`)
- Executing transactions (`apply_tx`)
- Managing block lifecycle (`do_begin_block` and `do_end_block`)
- Executing queries (`query`)
- Running privileged operations (`sudo`)

```mermaid
classDiagram
    class Stf {
        +sudo()
        +apply_block()
        +do_begin_block()
        +do_end_block()
        +apply_tx()
        +query()
    }
    
    class Invoker {
        -whoami: AccountId
        -sender: AccountId
        -funds: Vec<FungibleAsset>
        -account_codes: Rc<RefCell>
        -storage: Rc<RefCell<ExecutionState>>
        -gas_counter: Rc<RefCell<GasCounter>>
        +do_query()
        +do_exec()
        +handle_system_exec()
        +handle_storage_exec()
        +handle_event_handler_exec()
    }
    
    class ExecutionState {
        -base_storage: &S
        -overlay: HashMap
        -undo_log: Vec<StateChange>
        -events: Vec<Event>
        +get()
        +set()
        +remove()
        +checkpoint()
        +restore()
        +emit_event()
        +pop_events()
        +into_changes()
    }
    
    class GasCounter {
        +infinite()
        +finite()
        +consume_gas()
        +consume_get_gas()
        +consume_set_gas()
        +consume_remove_gas()
        +gas_used()
    }
    
    Stf --> Invoker: uses
    Invoker --> ExecutionState: manages
    Invoker --> GasCounter: tracks
```

### Execution State

The ExecutionState provides an abstraction over the blockchain's state with features for:

- Reading and writing state (`get`, `set`, `remove`)
- Creating checkpoints for transactional execution
- Rolling back state changes if needed
- Tracking emitted events

```mermaid
classDiagram
    class ExecutionState {
        -base_storage: ReadonlyKV
        -overlay: HashMap
        -undo_log: Vec<StateChange>
        -events: Vec<Event>
        +get()
        +set()
        +remove()
        +checkpoint()
        +restore()
        +emit_event()
        +into_changes()
    }
    
    class StateChange {
        +Set(key, previous_value)
        +Remove(key, previous_value)
        +revert()
    }
    
    class Checkpoint {
        +undo_log_index: usize
        +events_index: usize
    }
    
    ExecutionState --> StateChange: records
    ExecutionState --> Checkpoint: creates
```

### Gas System

The GasCounter tracks and enforces resource usage limits during execution:

- Infinite mode for system operations
- Finite mode for user transactions
- Configurable gas costs for different operations

```mermaid
classDiagram
    class GasCounter {
        +Infinite
        +Finite(gas_limit,gas_used,storage_gas_config)
        +infinite()
        +finite()
        +consume_gas()
        +consume_get_gas()
        +consume_set_gas()
        +consume_remove_gas()
        +gas_used()
    }
    
    class StorageGasConfig {
        +storage_get_charge: u64
        +storage_set_charge: u64
        +storage_remove_charge: u64
    }
    
    GasCounter --> StorageGasConfig: uses
```

### Invoker

The Invoker acts as an execution environment for actors in the system:

- Manages identity (`whoami` and `sender`)
- Tracks funds being transferred
- Provides access to storage and gas accounting
- Handles special system accounts

## Execution Flow

### Block Execution Flow

```mermaid
sequenceDiagram
    participant External as External Component
    participant Stf as State Transition Function
    participant Invoker as Invoker
    participant ExecutionState as Execution State
    participant GasCounter as Gas Counter
    
    External->>Stf: apply_block(block)
    Stf->>Invoker: new_block_state()
    Stf->>Stf: do_begin_block()
    Stf->>ExecutionState: pop_events()
    
    loop For each transaction
        Stf->>Stf: apply_tx(tx)
        Stf->>Invoker: clone_with_gas()
        Stf->>Invoker: validate_tx()
        alt Validation Success
            Stf->>Invoker: branch_for_new_exec()
            Stf->>Invoker: do_exec()
            Invoker->>ExecutionState: get/set/remove
            Invoker->>GasCounter: consume_gas
        else Validation Error
            Stf->>ExecutionState: pop_events()
        end
    end
    
    Stf->>Stf: do_end_block()
    Stf->>ExecutionState: pop_events()
    Stf->>ExecutionState: into_state_changes()
    Stf-->>External: BlockResult
```

### Transaction Execution Flow

```mermaid
sequenceDiagram
    participant Stf as State Transition Function
    participant Invoker as Invoker
    participant ExecutionState as Execution State
    participant Account as Account Code
    participant GasCounter as Gas Counter
    Stf ->> Invoker: apply_tx(tx)
    Invoker ->> Invoker: handle_transfers()
    Invoker ->> ExecutionState: checkpoint()

    alt System Account
        Invoker ->> Invoker: handle_system_exec()
    else Storage Account
        Invoker ->> Invoker: handle_storage_exec()
    else Event Handler
        Invoker ->> Invoker: handle_event_handler_exec()
    else Regular Account
        Invoker ->> Invoker: branch_exec()
        Invoker ->> Account: execute()
        Account ->> Invoker: do_query/do_exec
    end

    alt Execution Error
        Invoker ->> ExecutionState: restore(checkpoint)
    end

    Invoker ->> GasCounter: gas_used()
    Invoker ->> ExecutionState: pop_events()
    Invoker -->> Stf: TxResult
```

## Storage and State Management

The system uses a layered approach to state management:

1. **Base Storage**: Persistent blockchain state (immutable within a transaction)
2. **Overlay**: In-memory overlay of changes made during execution
3. **Undo Log**: Record of changes for potential rollback

```mermaid
graph TD
    A[Client Request] --> B[Transaction]
    B --> C[State Transition Function]
    C --> D[Invoker]
    D --> E[ExecutionState]
    E --> F[Overlay]
    E --> G[Base Storage]
    
    F -- "Checkpoint/Restore" --> H[Undo Log]
    F -- "Commit" --> I[State Changes]
    I --> J[Persistent State]
```

## Special Account Types

The system has special system accounts that handle core functionality:

1. **Runtime Account**: Handles system operations like account creation
2. **Storage Account**: Provides storage operations to actors
3. **Event Handler**: Manages event emission and processing

```mermaid
graph TD
    A[Invoker] --> B[Account Type]
    B --> C[Runtime Account]
    B --> D[Storage Account]
    B --> E[Event Handler]
    B --> F[Regular Actor]
    C --> G[handle_system_exec]
    C --> H[handle_system_query]
    D --> I[handle_storage_exec]
    D --> J[handle_storage_query]
    E --> K[handle_event_handler_exec]
    F --> L[with_account/execute/query]
```

## Gas Model

The gas system ensures resource usage is properly accounted for:

```mermaid
flowchart TD
    A[Transaction] --> B{Finite or Infinite?}
    B -- Finite --> C[Gas Limit]
    B -- Infinite --> D[No Limit]
    
    C --> E[Track Gas Usage]
    E --> F{Exceeds Limit?}
    F -- Yes --> G[Error: Out of Gas]
    F -- No --> H[Continue Execution]
    
    D --> H
    
    H --> I[Storage Operation]
    I --> J[Compute Operation]
    I --> K[consume_get_gas]
    I --> L[consume_set_gas]
    I --> M[consume_remove_gas]
    J --> N[consume_gas]
```

## Results and Event Handling

The system produces structured results from execution:

```mermaid
classDiagram
    class BlockResult {
        +begin_block_events: Vec<Event>
        +state_changes: Vec<StateChange>
        +tx_results: Vec<TxResult>
        +end_block_events: Vec<Event>
    }
    
    class TxResult {
        +events: Vec<Event>
        +gas_used: u64
        +response: SdkResult<InvokeResponse>
    }
    
    class Event {
        +type: String
        +attributes: Vec<KeyValue>
    }
    
    class StateChange {
        +Set (key, value)
        +Remove (key)
    }
    
    BlockResult --> TxResult: contains
    BlockResult --> StateChange: contains
    TxResult --> Event: contains
```

## Conclusion

The Application Actor system provides a robust framework for executing transactions, managing state, and enforcing
resource limits in the CoTend/Evolve blockchain. Its modular design with the STF, Invoker, ExecutionState, and
GasCounter components enables secure and efficient execution of smart contracts in an actor-based model.
