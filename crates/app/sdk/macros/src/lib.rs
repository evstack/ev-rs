extern crate proc_macro;

use heck::ToUpperCamelCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use sha2::{Digest, Sha256};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, FnArg, Ident, ImplItem, Item, ItemImpl,
    ItemMod, ItemTrait, Pat, ReturnType, Signature, TraitItem, Type, TypePath,
};

/// This attribute macro generates code for smart contracts in the Evolve SDK, including:
/// 1) Message types (e.g. `InitializeMsg`, `TransferMsg`, etc.) that define contract interfaces
/// 2) An `impl AccountCode` for the target struct/impl (if found)
/// 3) A "wrapper account" struct (e.g. `AssetAccount`) that has convenience methods
///
/// It handles payable and non-payable functions:
/// - For **non-payable** `init/exec`, auto-injects a funds check
/// - For **payable** `init/exec`, no check is injected
/// - For **wrapper** code with payable functions, adds an extra `funds: Vec<FungibleAsset>` argument
/// - **Queries** can never be `payable`
#[proc_macro_attribute]
pub fn account_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse inputs
    let account_ident = parse_macro_input!(attr as Ident);
    let mut module = parse_macro_input!(item as ItemMod);

    // Extract module content
    let content = match get_module_content(&mut module) {
        Ok(content) => content,
        Err(err) => return err.to_compile_error().into(),
    };

    // Process the module and generate code
    match process_account_impl(&account_ident, content) {
        Ok(()) => TokenStream::from(quote! { #module }),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Main processing function for account_impl that orchestrates:
/// 1. Collecting annotated functions from implementation and trait
/// 2. Merging these functions across sources
/// 3. Generating code components
/// 4. Extending the module with generated code
fn process_account_impl(account_ident: &Ident, content: &mut Vec<Item>) -> Result<(), syn::Error> {
    // Collect the annotated functions
    let (maybe_impl_info, trait_info) = collect_annotated_items(account_ident, content)?;

    // Merge sets of init/exec/query functions
    let init_fn = merge_init(
        &maybe_impl_info.as_ref().and_then(|f| f.init_fn.clone()),
        &trait_info.init_fn,
    )?;

    let exec_fns = merge_functions(
        maybe_impl_info
            .as_ref()
            .map(|f| &f.exec_fns[..])
            .unwrap_or(&[]),
        &trait_info.exec_fns,
    )?;

    let query_fns = merge_functions(
        maybe_impl_info
            .as_ref()
            .map(|f| &f.query_fns[..])
            .unwrap_or(&[]),
        &trait_info.query_fns,
    )?;

    // Generate code
    let generated_items = generate_code_components(
        account_ident,
        &init_fn,
        &exec_fns,
        &query_fns,
        maybe_impl_info.is_some(),
    )?;

    // Add generated items to the module
    content.extend(generated_items);

    Ok(())
}

/// Generates all necessary code components and returns them as a vector of Items
/// that can be added to the module.
fn generate_code_components(
    account_ident: &Ident,
    init_fn: &Option<FunctionInfo>,
    exec_fns: &[FunctionInfo],
    query_fns: &[FunctionInfo],
    has_impl: bool,
) -> Result<Vec<Item>, syn::Error> {
    // Generate all code
    let code = generate_all_code(account_ident, init_fn, exec_fns, query_fns, has_impl)?;

    // Parse into items
    let file: syn::File = syn::parse2(code)?;
    Ok(file.items)
}

/// Generates all code components as a TokenStream, including:
/// 1. Message structs for init/exec/query functions
/// 2. AccountCode implementation (if applicable)
/// 3. Wrapper struct with convenience methods
fn generate_all_code(
    account_ident: &Ident,
    init_fn: &Option<FunctionInfo>,
    exec_fns: &[FunctionInfo],
    query_fns: &[FunctionInfo],
    has_impl: bool,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    // 1) Generate message structs
    let mut generated_msgs = Vec::new();
    if let Some(ref info) = init_fn {
        generated_msgs.push(generate_msg_struct(info));
    }
    for info in exec_fns {
        generated_msgs.push(generate_msg_struct(info));
    }
    for info in query_fns {
        generated_msgs.push(generate_msg_struct(info));
    }

    // 2) Generate AccountCode implementation if needed
    let accountcode_impl = if has_impl {
        generate_accountcode_impl(account_ident, init_fn, exec_fns, query_fns)
    } else {
        quote! {}
    };

    // 3) Generate wrapper struct and implementation
    let wrapper_struct = generate_wrapper_struct(account_ident, init_fn, exec_fns, query_fns);

    // Combine all components
    Ok(quote! {
        #(#generated_msgs)*
        #accountcode_impl
        #wrapper_struct
    })
}

/// Represents the kind of function (init, exec, query) and whether it can receive funds.
#[derive(Clone, PartialEq)]
enum FunctionKind {
    /// Initialization function that may or may not be payable
    Init { payable: bool },
    /// Execution function that may or may not be payable
    Exec { payable: bool },
    /// Query function (never payable)
    Query,
}

/// Stores detailed information about an annotated function discovered in the code.
#[derive(Clone)]
struct FunctionInfo {
    /// The original function name from the code
    fn_name: Ident,
    /// The generated message struct name (e.g., InitializeMsg)
    msg_name: Ident,
    /// What kind of function this is (init/exec/query + payable status)
    kind: FunctionKind,
    /// Parameters extracted from the function signature (for message fields)
    params: Vec<(Ident, Type)>,
    /// Return type (the T in `-> SdkResult<T>`)
    return_type: Type,
}

/// Holds the collected annotated functions from an impl block or trait definition.
struct CollectedInfo {
    /// The single initialization function (if any)
    init_fn: Option<FunctionInfo>,
    /// All execution functions
    exec_fns: Vec<FunctionInfo>,
    /// All query functions
    query_fns: Vec<FunctionInfo>,
}

/// Extracts the contents of a module, ensuring it's an inline module with braces.
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

// -----------------------------------------
//  Collecting Annotated Functions
// -----------------------------------------

/// Collects all annotated functions from both implementations and trait definitions
/// within the module that match the target account identifier.
///
/// Returns a tuple of:
/// - Option<CollectedInfo> from impl blocks (if any)
/// - CollectedInfo from trait definitions
fn collect_annotated_items(
    account_ident: &Ident,
    items: &mut Vec<Item>,
) -> Result<(Option<CollectedInfo>, CollectedInfo), syn::Error> {
    let mut impl_info: Option<CollectedInfo> = None;
    let mut trait_info = CollectedInfo {
        init_fn: None,
        exec_fns: vec![],
        query_fns: vec![],
    };

    for item in items {
        match item {
            Item::Impl(imp) => {
                if is_impl_for_account(account_ident, imp) {
                    let ci = collect_from_impl(imp)?;
                    if let Some(ref mut existing) = impl_info {
                        merge_collected_info(existing, ci)?;
                    } else {
                        impl_info = Some(ci);
                    }
                }
            }
            Item::Trait(tr) => {
                if tr.ident == *account_ident {
                    let ci = collect_from_trait(tr)?;
                    merge_collected_info(&mut trait_info, ci)?;
                }
            }
            _ => {}
        }
    }

    Ok((impl_info, trait_info))
}

/// Collects annotated methods from an implementation block, categorizing them
/// into init, exec, and query functions based on their attributes.
fn collect_from_impl(impl_block: &ItemImpl) -> Result<CollectedInfo, syn::Error> {
    let mut collected = CollectedInfo {
        init_fn: None,
        exec_fns: vec![],
        query_fns: vec![],
    };

    for impl_item in &impl_block.items {
        let method = match impl_item {
            ImplItem::Fn(m) => m,
            _ => continue,
        };
        if let Some(fi) = extract_function_info(&method.sig, &method.attrs)? {
            insert_function_info(&mut collected, fi)?;
        }
    }
    Ok(collected)
}

/// Collects annotated methods from a trait definition, categorizing them
/// into init, exec, and query functions.
fn collect_from_trait(trait_def: &ItemTrait) -> Result<CollectedInfo, syn::Error> {
    let mut collected = CollectedInfo {
        init_fn: None,
        exec_fns: vec![],
        query_fns: vec![],
    };

    for item in &trait_def.items {
        let TraitItem::Fn(m) = item else { continue };
        if let Some(fi) = extract_function_info(&m.sig, &m.attrs)? {
            insert_function_info(&mut collected, fi)?;
        }
    }
    Ok(collected)
}

/// Inserts a function info into the appropriate collection based on its kind.
/// Ensures there is only one init function.
fn insert_function_info(collected: &mut CollectedInfo, fi: FunctionInfo) -> Result<(), syn::Error> {
    match fi.kind {
        FunctionKind::Init { .. } => {
            if collected.init_fn.is_some() {
                return Err(syn::Error::new(
                    fi.fn_name.span(),
                    "Multiple #[init] functions are not allowed.",
                ));
            }
            collected.init_fn = Some(fi);
        }
        FunctionKind::Exec { .. } => {
            collected.exec_fns.push(fi);
        }
        FunctionKind::Query => {
            collected.query_fns.push(fi);
        }
    }
    Ok(())
}

/// Merges function collections from different sources, handling potential conflicts.
/// For init functions, ensures there is only one.
fn merge_collected_info(
    target: &mut CollectedInfo,
    source: CollectedInfo,
) -> Result<(), syn::Error> {
    // Merge init
    if target.init_fn.is_some() {
        if let Some(init) = source.init_fn {
            return Err(syn::Error::new(
                init.fn_name.span(),
                "Multiple #[init] found.",
            ));
        }
    }
    if let Some(init) = source.init_fn {
        target.init_fn = Some(init);
    }
    // Merge exec
    target.exec_fns.extend(source.exec_fns);
    // Merge query
    target.query_fns.extend(source.query_fns);
    Ok(())
}

// -----------------------------------------
//  Attribute Parsing
// -----------------------------------------

/// Extracts function information from a method signature if it has one of the
/// recognized attributes (#[init], #[exec], or #[query]).
///
/// Validates parameter structure and extracts relevant details for code generation.
fn extract_function_info(
    sig: &Signature,
    attrs: &[Attribute],
) -> Result<Option<FunctionInfo>, syn::Error> {
    let kind = match parse_function_kind(attrs)? {
        Some(k) => k,
        None => return Ok(None),
    };

    let fn_name = sig.ident.clone();
    let msg_name = format_ident!("{}Msg", fn_name.to_string().to_upper_camel_case());

    // Basic param checks
    let inputs: Vec<_> = sig.inputs.iter().collect();
    if inputs.len() < 2 {
        return Err(syn::Error::new(
            sig.ident.span(),
            "Expected at least two parameters: a receiver (&self) and an environment",
        ));
    }
    if !matches!(inputs[0], FnArg::Receiver(_)) {
        return Err(syn::Error::new(
            inputs[0].span(),
            "Expected first param to be &self or &mut self",
        ));
    }

    // Gather the "middle" params as message fields. The last param we assume is `env`.
    let field_inputs = &inputs[1..inputs.len() - 1];
    let mut params = Vec::new();
    for input in field_inputs {
        match input {
            FnArg::Typed(pat_type) => {
                let ident = match &*pat_type.pat {
                    Pat::Ident(pi) => pi.ident.clone(),
                    _ => {
                        return Err(syn::Error::new(
                            pat_type.span(),
                            "Only simple identifier patterns are supported",
                        ));
                    }
                };
                params.push((ident, (*pat_type.ty).clone()));
            }
            FnArg::Receiver(_) => {
                return Err(syn::Error::new(
                    input.span(),
                    "Unexpected `self` in the middle of function parameters",
                ));
            }
        }
    }

    let return_type = parse_return_type(sig)?;
    Ok(Some(FunctionInfo {
        fn_name,
        msg_name,
        kind,
        params,
        return_type,
    }))
}

/// Parses the return type of a function, expecting `-> SdkResult<T>`
/// and extracting the inner type T.
fn parse_return_type(sig: &Signature) -> Result<Type, syn::Error> {
    match &sig.output {
        ReturnType::Default => Ok(syn::parse_quote! { () }),
        ReturnType::Type(_, ty) => {
            if let Type::Path(TypePath { path, .. }) = &**ty {
                if let Some(seg) = path.segments.last() {
                    if seg.ident == "SdkResult" {
                        if let syn::PathArguments::AngleBracketed(generic_args) = &seg.arguments {
                            if let Some(syn::GenericArgument::Type(inner_ty)) =
                                generic_args.args.first()
                            {
                                return Ok(inner_ty.clone());
                            }
                        }
                    }
                }
            }
            Err(syn::Error::new(
                sig.output.span(),
                "Expected return type of the form -> SdkResult<T>",
            ))
        }
    }
}

/// Determines the function kind by examining attributes, looking for #[init],
/// #[exec], or #[query], and detects if init/exec are marked as payable.
fn parse_function_kind(attrs: &[Attribute]) -> Result<Option<FunctionKind>, syn::Error> {
    for attr in attrs {
        let Some(ident) = attr.path().get_ident() else {
            continue;
        };

        match ident.to_string().as_str() {
            "exec" => {
                // If the user wrote `#[exec]` => parse_is_payable returns false
                // If the user wrote `#[exec(payable)]` => returns true
                let is_payable = parse_is_payable(attr)?;
                return Ok(Some(FunctionKind::Exec {
                    payable: is_payable,
                }));
            }
            "init" => {
                let is_payable = parse_is_payable(attr)?;
                return Ok(Some(FunctionKind::Init {
                    payable: is_payable,
                }));
            }
            "query" => {
                // If `#[query(...)]`, we can handle or forbid it
                let meta = &attr.meta;
                if matches!(meta, syn::Meta::List(_)) {
                    return Err(syn::Error::new(
                        attr.span(),
                        "`#[query(...)]` cannot have arguments; queries cannot be payable",
                    ));
                }
                return Ok(Some(FunctionKind::Query));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Determines if a function is marked as payable by examining its attribute format.
/// Returns true only if the attribute contains the explicit `payable` parameter.
fn parse_is_payable(attr: &Attribute) -> Result<bool, syn::Error> {
    let meta = &attr.meta;

    match meta {
        // e.g. #[exec] with no parentheses
        syn::Meta::Path(_) => Ok(false),

        // e.g. #[exec(...)]
        syn::Meta::List(list) => {
            let mut payable = false;

            // Now parse the contents inside ( ... ) via parse_nested_meta
            list.parse_nested_meta(|nested| {
                if nested.path.is_ident("payable") {
                    payable = true;
                    Ok(())
                } else {
                    Err(syn::Error::new(
                        nested.path.span(),
                        "Unsupported attribute argument; expected `payable`",
                    ))
                }
            })?;

            Ok(payable)
        }

        // e.g. #[exec = something], treat as no parentheses or error if you want
        syn::Meta::NameValue(_) => Ok(false),
    }
}

// -----------------------------------------
//  Checking if `impl` is for the right type
// -----------------------------------------

/// Checks if an implementation block is for the account type we're targeting.
fn is_impl_for_account(account_ident: &Ident, impl_block: &ItemImpl) -> bool {
    if let Type::Path(tp) = &*impl_block.self_ty {
        if let Some(seg) = tp.path.segments.last() {
            return seg.ident == *account_ident;
        }
    }
    false
}

// -----------------------------------------
//  Merging logic
// -----------------------------------------

/// Merges two optional init functions, ensuring there is only one.
fn merge_init(
    lhs: &Option<FunctionInfo>,
    rhs: &Option<FunctionInfo>,
) -> Result<Option<FunctionInfo>, syn::Error> {
    match (lhs, rhs) {
        (None, None) => Ok(None),
        (Some(l), None) => Ok(Some(l.clone())),
        (None, Some(r)) => Ok(Some(r.clone())),
        (Some(_), Some(r)) => Err(syn::Error::new(r.fn_name.span(), "Multiple #[init] found.")),
    }
}

/// Merges two vectors of exec or query functions.
fn merge_functions(
    lhs: &[FunctionInfo],
    rhs: &[FunctionInfo],
) -> Result<Vec<FunctionInfo>, syn::Error> {
    let mut out = lhs.to_vec();
    out.extend_from_slice(rhs);
    Ok(out)
}

// -----------------------------------------
// 1) Generate each message struct
// -----------------------------------------

/// Generates a message struct for a function along with any necessary trait implementations.
/// The generated struct will have fields corresponding to the function parameters.
fn generate_msg_struct(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let msg_name = &info.msg_name;
    let fn_name_str = info.fn_name.to_string();
    let fields = info.params.iter().map(|(ident, ty)| {
        quote! { pub #ident: #ty, }
    });

    // Generate an 8-byte ID from the function name
    let fn_id = compute_function_id(&fn_name_str);

    match info.kind {
        FunctionKind::Init { .. } => quote! {
            #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize, ::core::clone::Clone)]
            pub struct #msg_name {
                #(#fields)*
            }
        },
        FunctionKind::Exec { .. } | FunctionKind::Query => {
            quote! {
                #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize, ::core::clone::Clone)]
                pub struct #msg_name {
                    #(#fields)*
                }

                impl ::evolve_core::InvokableMessage for #msg_name {
                    const FUNCTION_IDENTIFIER: u64 = #fn_id;
                    const FUNCTION_IDENTIFIER_NAME: &'static str = #fn_name_str;
                }
            }
        }
    }
}

/// Computes a deterministic function ID based on SHA-256 hash of the function name.
/// Used for message dispatch in the runtime.
fn compute_function_id(fn_name: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(fn_name.as_bytes());
    let hash = hasher.finalize();
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&hash[..8]);
    u64::from_le_bytes(arr)
}

// -----------------------------------------
// 2) Generate `impl AccountCode for {account_ident}`
// -----------------------------------------

/// Generates the implementation of AccountCode trait for the target struct.
/// This includes the init, execute, and query dispatch functions.
fn generate_accountcode_impl(
    account_ident: &Ident,
    init_fn: &Option<FunctionInfo>,
    exec_fns: &[FunctionInfo],
    query_fns: &[FunctionInfo],
) -> proc_macro2::TokenStream {
    let account_name_str = account_ident.to_string();

    let init_impl = if let Some(info) = init_fn {
        generate_init_arm(info)
    } else {
        quote! {
            fn init(&self, _env: &mut dyn ::evolve_core::Environment, _request: &::evolve_core::InvokeRequest)
                -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
            {
                Err(::evolve_core::ERR_UNKNOWN_FUNCTION)
            }
        }
    };

    let exec_impl = {
        let arms = exec_fns.iter().map(generate_exec_match_arm);
        quote! {
            fn execute(&self, env: &mut dyn ::evolve_core::Environment, request: &::evolve_core::InvokeRequest)
                -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
            {
                use ::evolve_core::InvokableMessage;

                match request.function() {
                    #(#arms,)*
                    _ => Err(::evolve_core::ERR_UNKNOWN_FUNCTION)
                }
            }
        }
    };

    let query_impl = {
        let arms = query_fns.iter().map(|info| {
            let fn_id = format_ident!("FUNCTION_IDENTIFIER");
            let msg_name = &info.msg_name;
            let fn_name = &info.fn_name;
            let args = info.params.iter().map(|(n, _)| quote!(msg.#n));
            quote! {
                #msg_name::#fn_id => {
                    let msg: #msg_name = request.get()?;
                    let resp = self.#fn_name(#(#args, )* env)?;
                    ::evolve_core::InvokeResponse::new(&resp)
                }
            }
        });
        quote! {
            fn query(&self, env: &mut dyn ::evolve_core::EnvironmentQuery, request: &::evolve_core::InvokeRequest)
                -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
            {
                use ::evolve_core::InvokableMessage;
                match request.function() {
                    #(#arms,)*
                    _ => Err(::evolve_core::ERR_UNKNOWN_FUNCTION)
                }
            }
        }
    };

    quote! {
        impl ::evolve_core::AccountCode for #account_ident {
            fn identifier(&self) -> String {
                #account_name_str.to_string()
            }
            #init_impl
            #exec_impl
            #query_impl
        }
    }
}

/// Generates the init implementation method with funds check if non-payable.
fn generate_init_arm(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let msg_name = &info.msg_name;
    let fn_name = &info.fn_name;
    let args = info.params.iter().map(|(n, _)| quote! { msg.#n });

    // If it's not payable, insert a check for `env.funds()`.
    let funds_check = match info.kind {
        FunctionKind::Init { payable } if !payable => {
            quote! {
                if !env.funds().is_empty() {
                    return Err(::evolve_core::ERR_NOT_PAYABLE);
                }
            }
        }
        _ => quote! {},
    };

    quote! {
        fn init(&self, env: &mut dyn ::evolve_core::Environment, request: &::evolve_core::InvokeRequest)
            -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
        {
            let msg: #msg_name = request.get()?;
            #funds_check
            let resp = self.#fn_name(#(#args, )* env)?;
            ::evolve_core::InvokeResponse::new(&resp)
        }
    }
}

/// Generates a match arm for an exec function, including funds check if non-payable.
fn generate_exec_match_arm(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let fn_id = format_ident!("FUNCTION_IDENTIFIER");
    let msg_name = &info.msg_name;
    let fn_name = &info.fn_name;
    let args = info.params.iter().map(|(n, _)| quote!(msg.#n));

    // If not payable, we do the check. We'll inject it right after decoding `msg`.
    let funds_check = match info.kind {
        FunctionKind::Exec { payable } if !payable => {
            quote! {
                if !env.funds().is_empty() {
                    return Err(::evolve_core::ERR_NOT_PAYABLE);
                }
            }
        }
        _ => quote! {},
    };

    quote! {
        #msg_name::#fn_id => {
            let msg: #msg_name = request.get()?;
            #funds_check
            let resp = self.#fn_name(#(#args, )* env)?;
            ::evolve_core::InvokeResponse::new(&resp)
        }
    }
}

// -----------------------------------------
// 3) Generate the "wrapper" struct + impl
// -----------------------------------------

/// Generates a wrapper struct with convenience methods for interacting with the account.
/// This includes methods that mirror the init, exec, and query functions of the account.
fn generate_wrapper_struct(
    account_ident: &Ident,
    init_fn: &Option<FunctionInfo>,
    exec_fns: &[FunctionInfo],
    query_fns: &[FunctionInfo],
) -> proc_macro2::TokenStream {
    let wrapper_ident = format_ident!("{}Ref", account_ident);

    // init wrapper method
    let init_method = init_fn
        .as_ref()
        .map(|fn_info| generate_init_wrapper(account_ident, fn_info));

    // exec methods
    let exec_methods = exec_fns.iter().map(generate_exec_wrapper);

    // query methods
    let query_methods = query_fns.iter().map(generate_query_wrapper);

    quote! {
        /// A generated "wrapper" struct that holds the account-id pointer
        /// and provides convenience methods for init/exec/query calls.
        #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize, ::core::clone::Clone, ::core::cmp::PartialEq, ::core::cmp::Eq, ::core::cmp::Ord, ::core::cmp::PartialOrd, ::core::marker::Copy)]
        pub struct #wrapper_ident(pub ::evolve_core::AccountId);

        impl #wrapper_ident {
            #init_method
            #( #exec_methods )*
            #( #query_methods )*
        }

        impl #wrapper_ident {
            pub const fn new(account_id: ::evolve_core::AccountId) -> Self {
                Self(account_id)
            }
        }

        impl ::core::convert::From<::evolve_core::AccountId> for #wrapper_ident {
            fn from(account_id: ::evolve_core::AccountId) -> Self {
                Self(account_id)
            }
        }
    }
}

/// Generates a wrapper method for an init function, handling the creation of new accounts.
fn generate_init_wrapper(account_ident: &Ident, info: &FunctionInfo) -> proc_macro2::TokenStream {
    let fn_name = &info.fn_name;
    let msg_name = &info.msg_name;
    let return_ty = &info.return_type;

    // if payable, we have `funds: Vec<FungibleAsset>` param; otherwise none
    let (funds_param, funds_arg) = match info.kind {
        FunctionKind::Init { payable: true } => (
            quote!(funds: ::std::vec::Vec<::evolve_core::FungibleAsset>,),
            quote!(funds),
        ),
        _ => (quote!(), quote!(::std::vec::Vec::new())),
    };

    let params_decl = info.params.iter().map(|(n, t)| quote! { #n: #t });
    let param_names = info.params.iter().map(|(n, _)| quote!(#n));

    quote! {
        pub fn #fn_name(
            #funds_param
            #( #params_decl, )*
            env: &mut dyn ::evolve_core::Environment
        ) -> ::evolve_core::SdkResult<(Self, #return_ty)> {
            let (acc_id, resp) = ::evolve_core::low_level::create_account(
                stringify!(#account_ident).to_string(),
                &#msg_name { #( #param_names, )* },
                #funds_arg,
                env,
            )?;
            Ok((acc_id.into(), resp))
        }
    }
}

/// Generates a wrapper method for an exec function, handling the execution of account methods.
fn generate_exec_wrapper(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let fn_name = &info.fn_name;
    let msg_name = &info.msg_name;
    let return_ty = &info.return_type;

    // If payable, add an extra param for `funds`.
    let (funds_param, funds_arg) = match info.kind {
        FunctionKind::Exec { payable: true } => (
            quote!(funds: ::std::vec::Vec<::evolve_core::FungibleAsset>,),
            quote!(funds),
        ),
        _ => (quote!(), quote!(::std::vec::Vec::new())),
    };

    let params_decl = info.params.iter().map(|(n, t)| quote! { #n: #t });
    let param_names = info.params.iter().map(|(n, _)| quote!(#n));

    quote! {
        pub fn #fn_name(
            &self,
            #funds_param
            #( #params_decl, )*
            env: &mut dyn ::evolve_core::Environment
        ) -> ::evolve_core::SdkResult<#return_ty> {
            ::evolve_core::low_level::exec_account(
                self.0,
                &#msg_name { #( #param_names, )* },
                #funds_arg,
                env,
            )
        }
    }
}

/// Generates a wrapper method for a query function, handling read-only queries to accounts.
fn generate_query_wrapper(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let fn_name = &info.fn_name;
    let msg_name = &info.msg_name;
    let return_ty = &info.return_type;

    let params_decl = info.params.iter().map(|(n, t)| quote! { #n: #t });
    let param_names = info.params.iter().map(|(n, _)| quote!(#n));

    quote! {
        pub fn #fn_name(
            &self,
            #( #params_decl, )*
            env: &mut dyn ::evolve_core::EnvironmentQuery
        ) -> ::evolve_core::SdkResult<#return_ty> {
            ::evolve_core::low_level::query_account(
                self.0,
                &#msg_name { #( #param_names, )* },
                env,
            )
        }
    }
}

// -----------------------------------------
// Marker attributes
// -----------------------------------------

/// Marks a function as the initialization entry point for the account.
/// Can be made payable with #[init(payable)]
#[proc_macro_attribute]
pub fn init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a function as an execution entry point for the account.
/// Can be made payable with #[exec(payable)]
#[proc_macro_attribute]
pub fn exec(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a function as a query entry point for the account.
/// Cannot be payable.
#[proc_macro_attribute]
pub fn query(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_compute_function_id() {
        // Test that function IDs are deterministic
        let id1 = compute_function_id("transfer");
        let id2 = compute_function_id("transfer");
        assert_eq!(id1, id2, "Same function name should produce the same ID");

        // Test that different function names produce different IDs
        let id1 = compute_function_id("transfer");
        let id2 = compute_function_id("mint");
        assert_ne!(
            id1, id2,
            "Different function names should produce different IDs"
        );
    }

    #[test]
    fn test_parse_is_payable() {
        // Test non-payable function (no parentheses)
        let attr: syn::Attribute = parse_quote!(#[exec]);
        assert!(
            !parse_is_payable(&attr).unwrap(),
            "Function without payable should be non-payable"
        );

        // Test payable function
        let attr: syn::Attribute = parse_quote!(#[exec(payable)]);
        assert!(
            parse_is_payable(&attr).unwrap(),
            "Function with payable should be payable"
        );
    }

    #[test]
    fn test_parse_function_kind() {
        // Test init function
        let attrs: Vec<syn::Attribute> = vec![parse_quote!(#[init])];
        let kind = parse_function_kind(&attrs).unwrap().unwrap();
        assert!(matches!(kind, FunctionKind::Init { payable: false }));

        // Test payable init function
        let attrs: Vec<syn::Attribute> = vec![parse_quote!(#[init(payable)])];
        let kind = parse_function_kind(&attrs).unwrap().unwrap();
        assert!(matches!(kind, FunctionKind::Init { payable: true }));

        // Test exec function
        let attrs: Vec<syn::Attribute> = vec![parse_quote!(#[exec])];
        let kind = parse_function_kind(&attrs).unwrap().unwrap();
        assert!(matches!(kind, FunctionKind::Exec { payable: false }));

        // Test query function
        let attrs: Vec<syn::Attribute> = vec![parse_quote!(#[query])];
        let kind = parse_function_kind(&attrs).unwrap().unwrap();
        assert!(matches!(kind, FunctionKind::Query));

        // Test no relevant attribute
        let attrs: Vec<syn::Attribute> = vec![parse_quote!(#[other_attr])];
        assert!(parse_function_kind(&attrs).unwrap().is_none());
    }

    #[test]
    fn test_is_impl_for_account() {
        // Create an impl block for the right type
        let impl_block: syn::ItemImpl = parse_quote! {
            impl TestAccount {
                fn test(&self) {}
            }
        };
        let account_ident = syn::Ident::new("TestAccount", proc_macro2::Span::call_site());
        assert!(is_impl_for_account(&account_ident, &impl_block));

        // Create an impl block for a different type
        let impl_block: syn::ItemImpl = parse_quote! {
            impl OtherAccount {
                fn test(&self) {}
            }
        };
        assert!(!is_impl_for_account(&account_ident, &impl_block));
    }
}
