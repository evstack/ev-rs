// account_macro/src/lib.rs

extern crate proc_macro;

// Import the CamelCase trait so that `.to_camel_case()` is available.
use heck::ToUpperCamelCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use sha2::{Digest, Sha256};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, FnArg, Ident, ImplItem, Item, ItemImpl,
    ItemMod, Pat, Type,
};

/// This attribute macro generates message types and an AccountCode trait implementation
/// for the given account type. It processes functions annotated with #[init], #[exec],
/// or #[query] inside the account module.
#[proc_macro_attribute]
pub fn account_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the account type identifier from the macro attribute.
    let account_ident = parse_macro_input!(attr as Ident);

    // Parse the input module.
    let mut module = parse_macro_input!(item as ItemMod);

    // Extract the module content (the items inside the module braces).
    let content = match get_module_content(&mut module) {
        Ok(content) => content,
        Err(err) => return err.to_compile_error().into(),
    };

    // Collect functions (with markers #[init], #[exec], #[query]) from the impl block(s)
    // for the given account type.
    let (init_fn, exec_fns, query_fns) = match collect_account_functions(&account_ident, content) {
        Ok(val) => val,
        Err(err) => return err.to_compile_error().into(),
    };

    // Generate the message struct definitions for each function.
    let mut generated_msgs = Vec::new();
    if let Some(ref init_info) = init_fn {
        generated_msgs.push(generate_msg_struct(init_info));
    }
    for info in &exec_fns {
        generated_msgs.push(generate_msg_struct(info));
    }
    for info in &query_fns {
        generated_msgs.push(generate_msg_struct(info));
    }

    // Generate the AccountCode trait implementation for the account type.
    let accountcode_impl =
        generate_accountcode_impl(&account_ident, &init_fn, &exec_fns, &query_fns);

    // Combine all the generated message tokens into one TokenStream.
    // Parse that stream as a syn::File to extract multiple items.
    let file: syn::File = match syn::parse2(quote! {
        #(#generated_msgs)*
    }) {
        Ok(file) => file,
        Err(e) => return e.to_compile_error().into(),
    };

    // Append the generated items to the module's content.
    content.extend(file.items);

    // Parse and append the AccountCode impl (which is a single item).
    let impl_item: Item = match syn::parse2(accountcode_impl) {
        Ok(item) => item,
        Err(e) => return e.to_compile_error().into(),
    };
    content.push(impl_item);

    // Return the modified module.
    TokenStream::from(quote! { #module })
}

/// Extracts the items from an inline module.
/// Returns an error if the module is not inline (i.e. if it uses a semicolon instead of braces).
fn get_module_content(module: &mut ItemMod) -> Result<&mut Vec<Item>, syn::Error> {
    if let Some((_, ref mut items)) = module.content {
        Ok(items)
    } else {
        Err(syn::Error::new(
            module.span(),
            "account_impl requires an inline module (with braces).",
        ))
    }
}

/// Holds information about a function annotated with #[init], #[exec], or #[query].
#[derive(Clone)]
struct FunctionInfo {
    /// The original function name.
    fn_name: Ident,
    /// The generated message struct name (e.g. `TransferMsg`).
    msg_name: Ident,
    /// The kind of function: init, exec, or query.
    kind: FunctionKind,
    /// The list of parameters (name and type) that will become the message fields.
    /// (Note that the first two parameters, the receiver and environment, are skipped.)
    params: Vec<(Ident, Type)>,
}

/// Represents the kind of a function.
#[derive(Clone, PartialEq)]
enum FunctionKind {
    Init,
    Exec,
    Query,
}

/// Iterates over all items in the module and collects functions from impl blocks that
/// implement the given account type. Only functions marked with #[init], #[exec], or #[query]
/// are collected.
///
/// Returns a tuple with an optional init function, a vector of exec functions, and a vector
/// of query functions.
fn collect_account_functions(
    account_ident: &Ident,
    items: &Vec<Item>,
) -> Result<(Option<FunctionInfo>, Vec<FunctionInfo>, Vec<FunctionInfo>), syn::Error> {
    let mut init_fn: Option<FunctionInfo> = None;
    let mut exec_fns = Vec::new();
    let mut query_fns = Vec::new();

    // Iterate over all items in the module.
    for item in items.iter() {
        // Fast exit for non-impl items.
        let impl_block = match item {
            Item::Impl(item_impl) => item_impl,
            _ => continue,
        };

        // Skip impl blocks that are not for our target account type.
        if !is_impl_for_account(account_ident, impl_block) {
            continue;
        }

        // Process each item in the impl block.
        for impl_item in &impl_block.items {
            // Fast exit for non-function items.
            let method = match impl_item {
                // In syn 2.0 the function methods are represented by the `Fn` variant.
                ImplItem::Fn(method) => method,
                _ => continue,
            };

            // Get the function marker (#[init], #[exec], or #[query]). If none is present, skip.
            let marker = match get_function_marker(method) {
                Some(marker) => marker,
                None => continue,
            };

            // Create a message struct name by converting the function name to UpperCamelCase and appending "Msg".
            let fn_name = method.sig.ident.clone();
            let msg_name = format_ident!("{}Msg", fn_name.to_string().to_upper_camel_case());

            // Collect all function parameters.
            let inputs: Vec<_> = method.sig.inputs.iter().collect();
            if inputs.len() < 2 {
                return Err(syn::Error::new(
                    method.sig.ident.span(),
                    "Expected at least two parameters: a receiver and an environment parameter",
                ));
            }

            // Ensure the first parameter is the receiver.
            if !matches!(inputs[0], FnArg::Receiver(_)) {
                return Err(syn::Error::new(
                    inputs[0].span(),
                    "Expected the first parameter to be a receiver (e.g. &self)",
                ));
            }

            // The last parameter is assumed to be the environment.
            let env_input = inputs.last().unwrap();
            match env_input {
                FnArg::Typed(pat_type) => {
                    // Check that the environment parameter is a reference.
                    if let syn::Type::Reference(type_ref) = &*pat_type.ty {
                        // For query functions, the environment must be an immutable reference.
                        if marker == FunctionKind::Query && type_ref.mutability.is_some() {
                            return Err(syn::Error::new(
                                env_input.span(),
                                "Queries must have non mutable env",
                            ));
                        }
                    } else {
                        return Err(syn::Error::new(
                            env_input.span(),
                            "Expected the environment parameter to be a reference",
                        ));
                    }
                }
                _ => {
                    return Err(syn::Error::new(
                        env_input.span(),
                        "Expected the environment parameter to be a typed parameter",
                    ));
                }
            }

            // All parameters between the receiver and the environment become message fields.
            let field_inputs = &inputs[1..inputs.len() - 1];
            let mut params = Vec::new();
            for input in field_inputs {
                let pat_type = match input {
                    FnArg::Typed(pat_type) => pat_type,
                    _ => {
                        return Err(syn::Error::new(
                            input.span(),
                            "Unexpected function parameter type",
                        ));
                    }
                };

                // Only simple identifier patterns are supported.
                let ident = match &*pat_type.pat {
                    Pat::Ident(pat_ident) => pat_ident.ident.clone(),
                    _ => {
                        return Err(syn::Error::new(
                            pat_type.span(),
                            "Only simple identifier patterns are supported",
                        ));
                    }
                };

                params.push((ident, (*pat_type.ty).clone()));
            }

            // Build the function info record.
            let info = FunctionInfo {
                fn_name,
                msg_name,
                kind: marker.clone(),
                params,
            };

            // Insert the function info into the appropriate collection.
            match marker {
                FunctionKind::Init => {
                    if init_fn.is_some() {
                        return Err(syn::Error::new(
                            method.sig.ident.span(),
                            "Multiple #[init] functions are not allowed",
                        ));
                    }
                    init_fn = Some(info);
                }
                FunctionKind::Exec => exec_fns.push(info),
                FunctionKind::Query => query_fns.push(info),
            }
        }
    }

    Ok((init_fn, exec_fns, query_fns))
}

/// Extracts the marker from a method if it is annotated with #[init], #[exec], or #[query].
fn get_function_marker(method: &syn::ImplItemFn) -> Option<FunctionKind> {
    for attr in &method.attrs {
        if is_marker_attr(attr, "init") {
            return Some(FunctionKind::Init);
        } else if is_marker_attr(attr, "exec") {
            return Some(FunctionKind::Exec);
        } else if is_marker_attr(attr, "query") {
            return Some(FunctionKind::Query);
        }
    }
    None
}

/// Checks whether the given impl block is for the specified account type.
fn is_impl_for_account(account_ident: &Ident, item_impl: &ItemImpl) -> bool {
    if let Type::Path(type_path) = &*item_impl.self_ty {
        if let Some(seg) = type_path.path.segments.last() {
            return seg.ident == *account_ident;
        }
    }
    false
}

/// Generates the message struct definition (as a TokenStream) for a given function.
/// For exec and query functions, it also implements associated constants.
fn generate_msg_struct(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let msg_name = &info.msg_name;
    let fn_name_str = info.fn_name.to_string();
    let fields = info.params.iter().map(|(name, ty)| {
        quote! {
            pub #name: #ty,
        }
    });

    // Compute a unique function identifier using SHA-256 (taking the first 8 bytes).
    let mut hasher = Sha256::new();
    hasher.update(fn_name_str.as_bytes());
    let hash = hasher.finalize();
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&hash[..8]);
    let fn_id = u64::from_le_bytes(arr);

    if info.kind == FunctionKind::Init {
        // For init functions, we only need the struct.
        quote! {
            #[derive(BorshSerialize, BorshDeserialize)]
            pub struct #msg_name {
                #(#fields)*
            }
        }
    } else {
        // For exec and query functions, add constants for the function name and identifier.
        quote! {
            #[derive(BorshSerialize, BorshDeserialize)]
            pub struct #msg_name {
                #(#fields)*
            }
            impl #msg_name {
                pub const NAME: &'static str = #fn_name_str;
                pub const FUNCTION_IDENTIFIER: u64 = #fn_id;
            }
        }
    }
}

/// Generates the AccountCode trait implementation for the account type.
/// It creates the `identifier()`, `init()`, `execute()`, and `query()` methods
/// using the auto-generated message structs.
fn generate_accountcode_impl(
    account_ident: &Ident,
    init_fn: &Option<FunctionInfo>,
    exec_fns: &Vec<FunctionInfo>,
    query_fns: &Vec<FunctionInfo>,
) -> proc_macro2::TokenStream {
    let account_name_str = account_ident.to_string();

    // Generate the init branch.
    let init_impl = if let Some(info) = init_fn {
        let msg_name = &info.msg_name;
        let fn_name = &info.fn_name;
        // Generate arguments from the message fields.
        let args = info.params.iter().map(|(name, _)| quote! { msg.#name });
        quote! {
            fn init(&self, env: &mut dyn Environment, request: InvokeRequest) -> SdkResult<InvokeResponse> {
                use evolve_core::encoding::{Decodable, Encodable};
                let msg = request.decode::<#msg_name>()?;
                // Pass message fields first, then env as the last argument.
                let resp = self.#fn_name(#(#args),*, env)?;
                let msg_resp = Message::from(resp.encode()?);
                Ok(InvokeResponse::new(msg_resp))
            }
        }
    } else {
        quote! {
            fn init(&self, _env: &mut dyn Environment, _request: InvokeRequest) -> SdkResult<InvokeResponse> {
                Err(ERR_UNKNOWN_FUNCTION)
            }
        }
    };

    // Generate match arms for exec functions.
    let exec_match_arms = exec_fns.iter().map(|info| {
        let msg_name = &info.msg_name;
        let fn_name = &info.fn_name;
        let args = info.params.iter().map(|(name, _)| quote! { msg.#name });
        quote! {
            #msg_name::FUNCTION_IDENTIFIER => {
                let msg = request.decode::<#msg_name>()?;
                // Pass message fields first, then env as the last argument.
                let resp = self.#fn_name(#(#args),*, env)?;
                InvokeResponse::try_from_encodable(resp)
            }
        }
    });
    let exec_impl = quote! {
        fn execute(&self, env: &mut dyn Environment, request: InvokeRequest) -> SdkResult<InvokeResponse> {
            use evolve_core::encoding::Decodable;
            match request.function() {
                #(#exec_match_arms,)*
                _ => Err(ERR_UNKNOWN_FUNCTION)
            }
        }
    };

    // Generate match arms for query functions.
    let query_match_arms = query_fns.iter().map(|info| {
        let msg_name = &info.msg_name;
        let fn_name = &info.fn_name;
        let args = info.params.iter().map(|(name, _)| quote! { msg.#name });
        quote! {
            #msg_name::FUNCTION_IDENTIFIER => {
                let msg = request.decode::<#msg_name>()?;
                // Pass message fields first, then env as the last argument.
                let resp = self.#fn_name(#(#args),*, env)?;
                InvokeResponse::try_from_encodable(resp)
            }
        }
    });
    let query_impl = quote! {
        fn query(&self, env: &dyn Environment, request: InvokeRequest) -> SdkResult<InvokeResponse> {
            use evolve_core::encoding::Decodable;
            match request.function() {
                #(#query_match_arms,)*
                _ => Err(ERR_UNKNOWN_FUNCTION)
            }
        }
    };

    // Combine all parts into the final AccountCode implementation.
    quote! {
        impl AccountCode for #account_ident {
            fn identifier(&self) -> String {
                #account_name_str.to_string()
            }
            #init_impl
            #exec_impl
            #query_impl
        }
    }
}

/// Checks whether an attribute has the given identifier (e.g. "init", "exec", or "query").
fn is_marker_attr(attr: &Attribute, marker: &str) -> bool {
    attr.path()
        .get_ident()
        .map_or(false, |ident| ident == marker)
}

/// A no-op attribute used only as a marker.
#[proc_macro_attribute]
pub fn init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// A no-op attribute used only as a marker.
#[proc_macro_attribute]
pub fn exec(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// A no-op attribute used only as a marker.
#[proc_macro_attribute]
pub fn query(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
