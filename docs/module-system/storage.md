# Storage Collections

Evolve provides type-safe, deterministic storage collections for module state.

## Storage Prefix System

Each storage field needs a unique `u8` prefix within its module. The `#[derive(AccountState)]` macro validates uniqueness at compile time.

```rust
#[derive(evolve_core::AccountState)]
pub struct MyModule {
    #[storage(0)]
    pub counter: Item<u64>,
    #[storage(1)]
    pub balances: Map<AccountId, u128>,
    #[storage(2)]  // ERROR: compile fails if you use duplicate prefix
    pub metadata: Item<String>,
}
```

### Why Explicit Prefixes?

1. **Prevents accidental collisions** - Compile-time validation catches duplicates
2. **Stable across refactors** - Reordering fields doesn't change storage layout
3. **Migration safety** - Adding new fields doesn't corrupt existing data

### Multi-Prefix Collections

Some collections require multiple prefixes:

| Collection | Prefixes Required | Example |
|------------|------------------|---------|
| `Item<T>` | 1 | `Item::new(0)` |
| `Map<K,V>` | 1 | `Map::new(0)` |
| `Vector<T>` | 2 | `Vector::new(0, 1)` |
| `Queue<T>` | 2 | `Queue::new(0, 1)` |
| `UnorderedMap<K,V>` | 4 | `UnorderedMap::new(0, 1, 2, 3)` |

For multi-prefix collections, use manual initialization:

```rust
pub struct ComplexModule {
    validators: UnorderedMap<AccountId, Validator>,
    changes: Vector<Change>,
}

impl ComplexModule {
    pub const fn new() -> Self {
        Self {
            validators: UnorderedMap::new(0, 1, 2, 3),
            changes: Vector::new(4, 5),
        }
    }
}
```

## Collection Types

### Item<T>

Single value storage.

```rust
use evolve_collections::item::Item;

#[storage(0)]
pub counter: Item<u64>,

// Write
self.counter.set(&42, env)?;

// Read (returns error if not set)
let value = self.counter.get(env)?;

// Read (returns Option)
let maybe_value = self.counter.may_get(env)?;

// Update
self.counter.update(|current| {
    Ok(current.unwrap_or(0) + 1)
}, env)?;
```

### Map<K, V>

Key-value storage with deterministic iteration.

```rust
use evolve_collections::map::Map;

#[storage(1)]
pub balances: Map<AccountId, u128>,

// Write
self.balances.set(&account_id, &balance, env)?;

// Read
let balance = self.balances.get(&account_id, env)?;
let maybe_balance = self.balances.may_get(&account_id, env)?;

// Check existence
let exists = self.balances.exists(&account_id, env)?;

// Update
self.balances.update(&account_id, |current| {
    Ok(current.unwrap_or(0) + amount)
}, env)?;

// Remove
self.balances.remove(&account_id, env)?;
```

### Vector<T>

Ordered, indexable sequence.

```rust
use evolve_collections::vector::Vector;

// Requires 2 prefixes (index + elements)
pub items: Vector<Item>,

impl Module {
    pub const fn new() -> Self {
        Self { items: Vector::new(0, 1) }
    }
}

// Push
self.items.push(&item, env)?;

// Access by index
let item = self.items.get(0, env)?;

// Length
let len = self.items.len(env)?;

// Iterate
for item in self.items.iter(env)? {
    let item = item?;
    // process item
}
```

### Queue<T>

FIFO queue for ordered processing.

```rust
use evolve_collections::queue::Queue;

// Requires 2 prefixes
pub pending: Queue<Task>,

// Enqueue
self.pending.push_back(&task, env)?;

// Dequeue
let task = self.pending.pop_front(env)?;

// Peek
let next = self.pending.front(env)?;

// Check empty
let is_empty = self.pending.is_empty(env)?;
```

### UnorderedMap<K, V>

Hash-like map with O(1) access but deterministic iteration.

```rust
use evolve_collections::unordered_map::UnorderedMap;

// Requires 4 prefixes
pub validators: UnorderedMap<AccountId, Validator>,

impl Module {
    pub const fn new() -> Self {
        Self {
            validators: UnorderedMap::new(0, 1, 2, 3),
        }
    }
}

// Insert
self.validators.insert(&key, &value, env)?;

// Remove (swap-remove, O(1))
self.validators.remove(&key, env)?;

// Iterate (order not guaranteed but deterministic)
for entry in self.validators.iter(env)? {
    let (key, value) = entry?;
}
```

## Helper Fields

For stateless helper types (like `EventsEmitter`), use `#[skip_storage]`:

```rust
#[derive(evolve_core::AccountState)]
pub struct MyModule {
    #[storage(0)]
    pub data: Item<Data>,
    #[skip_storage]
    pub events: EventsEmitter,  // Initialized with Type::new()
}
```

## Storage Limits

| Limit | Value |
|-------|-------|
| Max key size | 254 bytes |
| Max value size | 1 MB |
| Max overlay entries | 100,000 per transaction |
| Max events | 10,000 per execution |

## Best Practices

1. **Use `may_get()` for optional data** - Returns `Option<T>` instead of erroring
2. **Prefer `update()` for read-modify-write** - Atomic operation
3. **Reserve prefix ranges** - Leave gaps for future fields (0, 10, 20...)
4. **Document prefix assignments** - Add comments for complex modules
