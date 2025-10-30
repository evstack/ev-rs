//! Schema types for module introspection and RPC discovery.
//!
//! This module provides type definitions that describe the structure of account
//! modules, their functions, and parameter types. These schemas are generated
//! by the `#[account_impl]` macro and can be exposed via RPC for client discovery.

use serde::{Deserialize, Serialize};

/// Describes a Rust type in a JSON-serializable format.
///
/// This enum represents the type system used in account module interfaces,
/// allowing RPC clients to understand parameter and return types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeSchema {
    /// A primitive type like u8, u64, bool, String.
    Primitive { name: String },
    /// An array/Vec type.
    Array { element: Box<TypeSchema> },
    /// An Option type.
    Optional { inner: Box<TypeSchema> },
    /// A tuple type.
    Tuple { elements: Vec<TypeSchema> },
    /// A struct type with named fields.
    Struct {
        name: String,
        fields: Vec<FieldSchema>,
    },
    /// An enum type with variants.
    Enum {
        name: String,
        variants: Vec<VariantSchema>,
    },
    /// The AccountId type.
    AccountId,
    /// The unit type ().
    Unit,
    /// An opaque type that couldn't be fully analyzed.
    /// Contains the Rust type path as a string for reference.
    Opaque { rust_type: String },
}

/// A named field with its type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldSchema {
    /// The field name.
    pub name: String,
    /// The field type.
    pub ty: TypeSchema,
}

/// An enum variant with optional fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VariantSchema {
    /// The variant name.
    pub name: String,
    /// The variant fields (empty for unit variants).
    pub fields: Vec<FieldSchema>,
}

/// Describes a function (init, exec, or query) in an account module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSchema {
    /// The function name.
    pub name: String,
    /// The function ID (hash of the function name).
    pub function_id: u64,
    /// The function kind (init, exec, or query).
    pub kind: FunctionKind,
    /// The function parameters (excluding self and env).
    pub params: Vec<FieldSchema>,
    /// The return type (the T in SdkResult<T>).
    pub return_type: TypeSchema,
    /// Whether this function can receive funds.
    pub payable: bool,
}

/// The kind of function in an account module.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FunctionKind {
    /// Initialization function (called once when account is created).
    Init,
    /// Execution function (modifies state).
    Exec,
    /// Query function (read-only).
    Query,
}

/// Complete schema for an account module.
///
/// Contains metadata and all function definitions for introspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSchema {
    /// The account type name (e.g., "Token").
    pub name: String,
    /// The account identifier string.
    pub identifier: String,
    /// The initialization function, if any.
    pub init: Option<FunctionSchema>,
    /// All execution functions.
    pub exec_functions: Vec<FunctionSchema>,
    /// All query functions.
    pub query_functions: Vec<FunctionSchema>,
}

impl AccountSchema {
    /// Creates a new AccountSchema builder.
    pub fn new(name: impl Into<String>, identifier: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            identifier: identifier.into(),
            init: None,
            exec_functions: Vec::new(),
            query_functions: Vec::new(),
        }
    }

    /// Sets the init function.
    pub fn with_init(mut self, init: FunctionSchema) -> Self {
        self.init = Some(init);
        self
    }

    /// Adds an exec function.
    pub fn with_exec(mut self, func: FunctionSchema) -> Self {
        self.exec_functions.push(func);
        self
    }

    /// Adds a query function.
    pub fn with_query(mut self, func: FunctionSchema) -> Self {
        self.query_functions.push(func);
        self
    }
}

impl FunctionSchema {
    /// Creates a new FunctionSchema.
    pub fn new(
        name: impl Into<String>,
        function_id: u64,
        kind: FunctionKind,
        return_type: TypeSchema,
    ) -> Self {
        Self {
            name: name.into(),
            function_id,
            kind,
            params: Vec::new(),
            return_type,
            payable: false,
        }
    }

    /// Adds a parameter to the function.
    pub fn with_param(mut self, name: impl Into<String>, ty: TypeSchema) -> Self {
        self.params.push(FieldSchema {
            name: name.into(),
            ty,
        });
        self
    }

    /// Sets the payable flag.
    pub fn with_payable(mut self, payable: bool) -> Self {
        self.payable = payable;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_schema_serialization() {
        let schema = TypeSchema::Array {
            element: Box::new(TypeSchema::Tuple {
                elements: vec![
                    TypeSchema::AccountId,
                    TypeSchema::Primitive {
                        name: "u128".into(),
                    },
                ],
            }),
        };

        let json = serde_json::to_string(&schema).unwrap();
        let parsed: TypeSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(schema, parsed);
    }

    #[test]
    fn test_account_schema_builder() {
        let schema = AccountSchema::new("Token", "Token")
            .with_init(
                FunctionSchema::new("initialize", 12345, FunctionKind::Init, TypeSchema::Unit)
                    .with_param(
                        "name",
                        TypeSchema::Primitive {
                            name: "String".into(),
                        },
                    ),
            )
            .with_query(
                FunctionSchema::new(
                    "get_balance",
                    67890,
                    FunctionKind::Query,
                    TypeSchema::Optional {
                        inner: Box::new(TypeSchema::Primitive {
                            name: "u128".into(),
                        }),
                    },
                )
                .with_param("account", TypeSchema::AccountId),
            );

        assert_eq!(schema.name, "Token");
        assert!(schema.init.is_some());
        assert_eq!(schema.query_functions.len(), 1);
    }

    #[test]
    fn test_function_kind_serialization() {
        let kind = FunctionKind::Query;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"query\"");
    }
}
