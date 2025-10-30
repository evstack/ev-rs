# Schema Introspection API

This document describes the schema introspection feature that allows RPC clients to discover all available modules and their query/exec functions at runtime.

## Overview

Evolve provides a reflection system that exposes module schemas via RPC. This enables:

- **Client SDK generation**: Automatically generate typed clients from schemas
- **Documentation**: Generate API documentation from live modules
- **Tooling**: Build explorers, debuggers, and development tools
- **Validation**: Validate requests against known schemas before sending

## Schema Structure

Each registered module exposes an `AccountSchema` containing:

```json
{
  "name": "Token",
  "identifier": "Token",
  "init": { /* FunctionSchema */ },
  "exec_functions": [ /* FunctionSchema[] */ ],
  "query_functions": [ /* FunctionSchema[] */ ]
}
```

### FunctionSchema

```json
{
  "name": "transfer",
  "function_id": 1234567890,
  "kind": "exec",
  "params": [
    { "name": "to", "ty": { "kind": "account_id" } },
    { "name": "amount", "ty": { "kind": "primitive", "name": "u128" } }
  ],
  "return_type": { "kind": "unit" },
  "payable": false
}
```

### TypeSchema Variants

| Kind | JSON Example | Description |
|------|--------------|-------------|
| `primitive` | `{"kind": "primitive", "name": "u128"}` | Basic types (u8-u128, bool, String) |
| `account_id` | `{"kind": "account_id"}` | Account identifier |
| `unit` | `{"kind": "unit"}` | Empty/void return |
| `array` | `{"kind": "array", "element": {...}}` | Vec<T> |
| `optional` | `{"kind": "optional", "inner": {...}}` | Option<T> |
| `tuple` | `{"kind": "tuple", "elements": [...]}` | (A, B, C) |
| `struct` | `{"kind": "struct", "name": "...", "fields": [...]}` | Named struct |
| `enum` | `{"kind": "enum", "name": "...", "variants": [...]}` | Enum type |
| `opaque` | `{"kind": "opaque", "rust_type": "..."}` | Unknown/complex type |

---

## JSON-RPC API

The schema introspection endpoints are in the `evolve` namespace.

### evolve_listModules

Returns all registered module identifiers.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "evolve_listModules",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": ["Token", "Scheduler", "EoaAccount"],
  "id": 1
}
```

### evolve_getModuleSchema

Returns the schema for a specific module.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "evolve_getModuleSchema",
  "params": ["Token"],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "name": "Token",
    "identifier": "Token",
    "init": {
      "name": "initialize",
      "function_id": 9876543210,
      "kind": "init",
      "params": [
        {"name": "metadata", "ty": {"kind": "opaque", "rust_type": "FungibleAssetMetadata"}},
        {"name": "balances", "ty": {"kind": "array", "element": {"kind": "tuple", "elements": [{"kind": "account_id"}, {"kind": "primitive", "name": "u128"}]}}},
        {"name": "supply_manager", "ty": {"kind": "optional", "inner": {"kind": "account_id"}}}
      ],
      "return_type": {"kind": "unit"},
      "payable": false
    },
    "exec_functions": [
      {
        "name": "transfer",
        "function_id": 1234567890,
        "kind": "exec",
        "params": [
          {"name": "to", "ty": {"kind": "account_id"}},
          {"name": "amount", "ty": {"kind": "primitive", "name": "u128"}}
        ],
        "return_type": {"kind": "unit"},
        "payable": false
      }
    ],
    "query_functions": [
      {
        "name": "get_balance",
        "function_id": 5555555555,
        "kind": "query",
        "params": [
          {"name": "account", "ty": {"kind": "account_id"}}
        ],
        "return_type": {"kind": "optional", "inner": {"kind": "primitive", "name": "u128"}},
        "payable": false
      }
    ]
  },
  "id": 1
}
```

**Response (module not found):**
```json
{
  "jsonrpc": "2.0",
  "result": null,
  "id": 1
}
```

### evolve_getAllSchemas

Returns schemas for all registered modules.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "evolve_getAllSchemas",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": [
    { "name": "Token", "identifier": "Token", ... },
    { "name": "Scheduler", "identifier": "Scheduler", ... }
  ],
  "id": 1
}
```

### curl Examples

```bash
# List all modules
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"evolve_listModules","params":[],"id":1}'

# Get Token schema
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"evolve_getModuleSchema","params":["Token"],"id":1}'

# Get all schemas
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"evolve_getAllSchemas","params":[],"id":1}'
```

---

## gRPC API

The schema introspection RPCs are part of the `ExecutionService`.

### Proto Definitions

```protobuf
service ExecutionService {
  // ... other RPCs ...

  // Schema introspection
  rpc ListModules(ListModulesRequest) returns (ListModulesResponse);
  rpc GetModuleSchema(GetModuleSchemaRequest) returns (GetModuleSchemaResponse);
  rpc GetAllSchemas(GetAllSchemasRequest) returns (GetAllSchemasResponse);
}

message ListModulesRequest {}
message ListModulesResponse {
  repeated string identifiers = 1;
}

message GetModuleSchemaRequest {
  string identifier = 1;
}
message GetModuleSchemaResponse {
  optional AccountSchema schema = 1;
}

message GetAllSchemasRequest {}
message GetAllSchemasResponse {
  repeated AccountSchema schemas = 1;
}
```

### Schema Message Types

```protobuf
message TypeSchema {
  oneof kind {
    string primitive = 1;
    TypeSchema array_element = 2;
    TypeSchema optional_inner = 3;
    TupleSchema tuple = 4;
    StructSchema struct_type = 5;
    EnumSchema enum_type = 6;
    bool account_id = 7;
    bool unit = 8;
    string opaque = 9;
  }
}

message FunctionSchema {
  string name = 1;
  uint64 function_id = 2;
  FunctionKind kind = 3;
  repeated FieldSchema params = 4;
  TypeSchema return_type = 5;
  bool payable = 6;
}

message AccountSchema {
  string name = 1;
  string identifier = 2;
  optional FunctionSchema init = 3;
  repeated FunctionSchema exec_functions = 4;
  repeated FunctionSchema query_functions = 5;
}
```

### grpcurl Examples

```bash
# List all modules
grpcurl -plaintext localhost:9545 evolve.v1.ExecutionService/ListModules

# Get Token schema
grpcurl -plaintext -d '{"identifier": "Token"}' \
  localhost:9545 evolve.v1.ExecutionService/GetModuleSchema

# Get all schemas
grpcurl -plaintext localhost:9545 evolve.v1.ExecutionService/GetAllSchemas
```

### Rust Client Example

```rust
use evolve_grpc::proto::evolve::v1::{
    execution_service_client::ExecutionServiceClient,
    ListModulesRequest, GetModuleSchemaRequest, GetAllSchemasRequest,
};

async fn discover_modules() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ExecutionServiceClient::connect("http://localhost:9545").await?;

    // List all modules
    let response = client.list_modules(ListModulesRequest {}).await?;
    println!("Modules: {:?}", response.into_inner().identifiers);

    // Get Token schema
    let response = client
        .get_module_schema(GetModuleSchemaRequest {
            identifier: "Token".to_string(),
        })
        .await?;

    if let Some(schema) = response.into_inner().schema {
        println!("Token has {} exec functions", schema.exec_functions.len());
        println!("Token has {} query functions", schema.query_functions.len());
    }

    Ok(())
}
```

---

## Server Setup

### Enabling Schema Introspection

To enable schema introspection, you need to provide an `AccountsCodeStorage` implementation when creating the `ChainStateProvider`.

#### Without Schema Support (default)

```rust
use evolve_chain_index::{ChainStateProvider, ChainStateProviderConfig, PersistentChainIndex};

// Creates provider with NoopAccountCodes (no schema support)
let provider = ChainStateProvider::new(
    Arc::new(index),
    ChainStateProviderConfig { chain_id: 1 },
);
```

#### With Schema Support

```rust
use evolve_chain_index::{ChainStateProvider, ChainStateProviderConfig};
use evolve_stf_traits::AccountsCodeStorage;

// Your AccountsCodeStorage implementation that holds registered modules
let account_codes: Arc<MyAccountCodes> = /* ... */;

let provider = ChainStateProvider::with_account_codes(
    Arc::new(index),
    ChainStateProviderConfig { chain_id: 1 },
    account_codes,
);
```

### Implementing AccountsCodeStorage

```rust
use std::collections::HashMap;
use std::sync::Arc;
use evolve_core::{AccountCode, ErrorCode};
use evolve_stf_traits::AccountsCodeStorage;

pub struct MyAccountCodes {
    codes: HashMap<String, Arc<dyn AccountCode>>,
}

impl MyAccountCodes {
    pub fn new() -> Self {
        let mut codes = HashMap::new();
        // Register your modules
        codes.insert("Token".to_string(), Arc::new(Token::default()) as Arc<dyn AccountCode>);
        codes.insert("Scheduler".to_string(), Arc::new(Scheduler::default()) as Arc<dyn AccountCode>);
        Self { codes }
    }
}

impl AccountsCodeStorage for MyAccountCodes {
    fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
    where
        F: FnOnce(Option<&dyn AccountCode>) -> R,
    {
        Ok(f(self.codes.get(identifier).map(|c| c.as_ref())))
    }

    fn list_identifiers(&self) -> Vec<String> {
        self.codes.keys().cloned().collect()
    }
}
```

---

## Use Cases

### 1. Generate TypeScript Client

```typescript
// Fetch schemas and generate types
const response = await fetch('http://localhost:8545', {
  method: 'POST',
  body: JSON.stringify({
    jsonrpc: '2.0',
    method: 'evolve_getAllSchemas',
    params: [],
    id: 1
  })
});

const { result: schemas } = await response.json();

for (const schema of schemas) {
  console.log(`Module: ${schema.name}`);
  for (const query of schema.query_functions) {
    console.log(`  Query: ${query.name}(${query.params.map(p => p.name).join(', ')})`);
  }
}
```

### 2. Build CLI Tool

```bash
#!/bin/bash
# List available queries for a module

MODULE=$1
SCHEMA=$(curl -s http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"evolve_getModuleSchema\",\"params\":[\"$MODULE\"],\"id\":1}" \
  | jq '.result')

echo "Queries for $MODULE:"
echo "$SCHEMA" | jq -r '.query_functions[] | "  \(.name): \(.params | map(.name) | join(", "))"'
```

### 3. Validate Requests

```rust
fn validate_query(schema: &AccountSchema, function_name: &str, params: &[Value]) -> Result<(), String> {
    let func = schema.query_functions
        .iter()
        .find(|f| f.name == function_name)
        .ok_or_else(|| format!("Unknown query: {}", function_name))?;

    if params.len() != func.params.len() {
        return Err(format!(
            "Expected {} params, got {}",
            func.params.len(),
            params.len()
        ));
    }

    Ok(())
}
```

---

## Function IDs

Each function has a deterministic `function_id` computed as the first 8 bytes of `SHA-256(function_name)`. This ID is used for message dispatch at runtime.

```rust
use sha2::{Sha256, Digest};

fn compute_function_id(name: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let hash = hasher.finalize();
    u64::from_le_bytes(hash[..8].try_into().unwrap())
}

// Example:
// compute_function_id("transfer") => 1234567890
// compute_function_id("get_balance") => 5555555555
```

The same function name always produces the same ID, enabling clients to cache and reuse IDs.
