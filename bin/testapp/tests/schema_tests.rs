//! Tests for schema generation via the account_impl macro.

// Testing code - determinism requirements do not apply.
#![allow(clippy::disallowed_types)]
#![allow(clippy::indexing_slicing)]

use std::collections::HashMap;
use std::sync::Arc;

use evolve_core::schema::{AccountSchema, FunctionKind, TypeSchema};
use evolve_core::{AccountCode, ErrorCode};
use evolve_scheduler::scheduler_account::Scheduler;
use evolve_stf_traits::AccountsCodeStorage;
use evolve_token::account::Token;

#[test]
fn test_token_schema_generation() {
    let token = Token::default();
    let schema = token.schema();

    assert_eq!(schema.name, "Token");
    assert_eq!(schema.identifier, "Token");

    // Verify init function exists
    assert!(schema.init.is_some());
    let init = schema.init.as_ref().unwrap();
    assert_eq!(init.name, "initialize");
    assert_eq!(init.kind, FunctionKind::Init);
    // Should have parameters: metadata, balances, supply_manager
    assert_eq!(init.params.len(), 3);
    assert_eq!(init.params[0].name, "metadata");
    assert_eq!(init.params[1].name, "balances");
    assert_eq!(init.params[2].name, "supply_manager");

    // Verify exec functions exist
    assert!(!schema.exec_functions.is_empty());
    let exec_names: Vec<_> = schema
        .exec_functions
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(exec_names.contains(&"mint"));
    assert!(exec_names.contains(&"burn"));
    assert!(exec_names.contains(&"transfer"));

    // Verify query functions exist
    assert!(!schema.query_functions.is_empty());
    let query_names: Vec<_> = schema
        .query_functions
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(query_names.contains(&"metadata"));
    assert!(query_names.contains(&"get_balance"));
    assert!(query_names.contains(&"total_supply"));
}

#[test]
fn test_token_schema_type_mapping() {
    let token = Token::default();
    let schema = token.schema();

    // Find the get_balance query
    let get_balance = schema
        .query_functions
        .iter()
        .find(|f| f.name == "get_balance")
        .expect("get_balance should exist");

    // Should have one parameter: account of type AccountId
    assert_eq!(get_balance.params.len(), 1);
    assert_eq!(get_balance.params[0].name, "account");
    assert_eq!(get_balance.params[0].ty, TypeSchema::AccountId);

    // Return type should be Optional<u128>
    match &get_balance.return_type {
        TypeSchema::Optional { inner } => match inner.as_ref() {
            TypeSchema::Primitive { name } => {
                assert_eq!(name, "u128");
            }
            _ => panic!("Expected Primitive inner type"),
        },
        _ => panic!("Expected Optional return type"),
    }
}

#[test]
fn test_schema_serialization() {
    let token = Token::default();
    let schema = token.schema();

    // Verify schema can be serialized to JSON
    let json = serde_json::to_string_pretty(&schema).expect("Schema should serialize");

    // Verify it can be deserialized back
    let parsed: AccountSchema = serde_json::from_str(&json).expect("Schema should deserialize");

    assert_eq!(parsed.name, schema.name);
    assert_eq!(parsed.identifier, schema.identifier);
    assert_eq!(parsed.exec_functions.len(), schema.exec_functions.len());
    assert_eq!(parsed.query_functions.len(), schema.query_functions.len());
}

// ============================================================================
// Integration tests for AccountsCodeStorage schema flow
// ============================================================================

/// Test AccountsCodeStorage that holds multiple modules
struct TestAccountCodes {
    codes: HashMap<String, Arc<dyn AccountCode>>,
}

impl TestAccountCodes {
    fn new() -> Self {
        let mut codes: HashMap<String, Arc<dyn AccountCode>> = HashMap::new();
        codes.insert("Token".to_string(), Arc::new(Token::default()));
        codes.insert("Scheduler".to_string(), Arc::new(Scheduler::default()));
        Self { codes }
    }
}

impl AccountsCodeStorage for TestAccountCodes {
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

#[test]
fn test_accounts_code_storage_list_identifiers() {
    let storage = TestAccountCodes::new();
    let mut identifiers = storage.list_identifiers();
    identifiers.sort();

    assert_eq!(identifiers.len(), 2);
    assert!(identifiers.contains(&"Token".to_string()));
    assert!(identifiers.contains(&"Scheduler".to_string()));
}

#[test]
fn test_accounts_code_storage_get_schema() {
    let storage = TestAccountCodes::new();

    // Get Token schema
    let token_schema = storage
        .with_code("Token", |code| code.map(|c| c.schema()))
        .expect("Should not error")
        .expect("Token should exist");

    assert_eq!(token_schema.name, "Token");
    assert!(token_schema.init.is_some());
    assert!(!token_schema.query_functions.is_empty());

    // Get Scheduler schema
    let scheduler_schema = storage
        .with_code("Scheduler", |code| code.map(|c| c.schema()))
        .expect("Should not error")
        .expect("Scheduler should exist");

    assert_eq!(scheduler_schema.name, "Scheduler");

    // Non-existent module returns None
    let missing = storage
        .with_code("NonExistent", |code| code.map(|c| c.schema()))
        .expect("Should not error");
    assert!(missing.is_none());
}

#[test]
fn test_accounts_code_storage_get_all_schemas() {
    let storage = TestAccountCodes::new();
    let identifiers = storage.list_identifiers();

    let schemas: Vec<AccountSchema> = identifiers
        .iter()
        .filter_map(|id| {
            storage
                .with_code(id, |code| code.map(|c| c.schema()))
                .ok()
                .flatten()
        })
        .collect();

    assert_eq!(schemas.len(), 2);

    let names: Vec<_> = schemas.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Token"));
    assert!(names.contains(&"Scheduler"));
}

#[test]
fn test_scheduler_schema_generation() {
    let scheduler = Scheduler::default();
    let schema = scheduler.schema();

    assert_eq!(schema.name, "Scheduler");
    assert_eq!(schema.identifier, "Scheduler");

    // Scheduler should have init
    assert!(schema.init.is_some());
    let init = schema.init.as_ref().unwrap();
    assert_eq!(init.name, "initialize");
    assert_eq!(init.kind, FunctionKind::Init);

    // Scheduler should have exec functions for begin/end block handling
    assert!(
        !schema.exec_functions.is_empty(),
        "Scheduler should have exec functions"
    );
}

#[test]
fn test_function_id_consistency() {
    // Verify that function IDs are consistent across calls
    let token1 = Token::default();
    let token2 = Token::default();

    let schema1 = token1.schema();
    let schema2 = token2.schema();

    // Same function should have same ID
    let get_balance1 = schema1
        .query_functions
        .iter()
        .find(|f| f.name == "get_balance");
    let get_balance2 = schema2
        .query_functions
        .iter()
        .find(|f| f.name == "get_balance");

    assert!(get_balance1.is_some());
    assert!(get_balance2.is_some());
    assert_eq!(
        get_balance1.unwrap().function_id,
        get_balance2.unwrap().function_id
    );
}

#[test]
fn test_schema_json_structure() {
    let token = Token::default();
    let schema = token.schema();
    let json = serde_json::to_value(&schema).expect("Should serialize to JSON");

    // Verify expected JSON structure
    assert!(json.is_object());
    assert_eq!(json["name"], "Token");
    assert_eq!(json["identifier"], "Token");
    assert!(json["init"].is_object());
    assert!(json["exec_functions"].is_array());
    assert!(json["query_functions"].is_array());

    // Verify init structure
    let init = &json["init"];
    assert!(init["name"].is_string());
    assert!(init["function_id"].is_number());
    assert_eq!(init["kind"], "init");
    assert!(init["params"].is_array());
    assert!(init["return_type"].is_object());

    // Verify a query function structure
    let queries = json["query_functions"].as_array().expect("Should be array");
    assert!(!queries.is_empty());

    let query = &queries[0];
    assert!(query["name"].is_string());
    assert!(query["function_id"].is_number());
    assert_eq!(query["kind"], "query");
}

#[test]
fn test_type_schema_variants() {
    let token = Token::default();
    let schema = token.schema();

    // Collect all type schemas from functions
    let mut type_kinds = std::collections::HashSet::new();

    if let Some(init) = &schema.init {
        collect_type_kinds(&init.return_type, &mut type_kinds);
        for param in &init.params {
            collect_type_kinds(&param.ty, &mut type_kinds);
        }
    }

    for func in &schema.exec_functions {
        collect_type_kinds(&func.return_type, &mut type_kinds);
        for param in &func.params {
            collect_type_kinds(&param.ty, &mut type_kinds);
        }
    }

    for func in &schema.query_functions {
        collect_type_kinds(&func.return_type, &mut type_kinds);
        for param in &func.params {
            collect_type_kinds(&param.ty, &mut type_kinds);
        }
    }

    // Should have various type kinds
    assert!(type_kinds.contains("unit") || type_kinds.contains("primitive"));
    assert!(type_kinds.contains("account_id"));
}

fn collect_type_kinds(ty: &TypeSchema, kinds: &mut std::collections::HashSet<String>) {
    match ty {
        TypeSchema::Primitive { .. } => {
            kinds.insert("primitive".to_string());
        }
        TypeSchema::Array { element } => {
            kinds.insert("array".to_string());
            collect_type_kinds(element, kinds);
        }
        TypeSchema::Optional { inner } => {
            kinds.insert("optional".to_string());
            collect_type_kinds(inner, kinds);
        }
        TypeSchema::Tuple { elements } => {
            kinds.insert("tuple".to_string());
            for elem in elements {
                collect_type_kinds(elem, kinds);
            }
        }
        TypeSchema::Struct { fields, .. } => {
            kinds.insert("struct".to_string());
            for field in fields {
                collect_type_kinds(&field.ty, kinds);
            }
        }
        TypeSchema::Enum { variants, .. } => {
            kinds.insert("enum".to_string());
            for variant in variants {
                for field in &variant.fields {
                    collect_type_kinds(&field.ty, kinds);
                }
            }
        }
        TypeSchema::AccountId => {
            kinds.insert("account_id".to_string());
        }
        TypeSchema::Unit => {
            kinds.insert("unit".to_string());
        }
        TypeSchema::Opaque { .. } => {
            kinds.insert("opaque".to_string());
        }
    }
}
