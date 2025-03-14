extern crate proc_macro;

use heck::ToUpperCamelCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use sha2::{Digest, Sha256};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, FnArg, Ident, ImplItem, Item, ItemImpl,
    ItemMod, ItemTrait, Pat, ReturnType, Signature, TraitItem, Type, TypePath,
};

/// This attribute macro generates:
/// 1) Message types (e.g. `InitializeMsg`, `TransferMsg`, etc.).
/// 2) An `impl AccountCode` for the target struct/impl (if found).
/// 3) A "wrapper account" struct (e.g. `AssetAccount`) that has convenience methods.
///
/// **Now** it also recognizes `(payable)` on `#[init(...)]` or `#[exec(...)]`:
/// - For **non-payable** `init/exec`, auto-injects `if !env.funds().is_empty() { ... }`.
/// - For **payable** `init/exec`, no check is injected.
/// - For **wrapper** code, if payable then an extra `funds: Vec<FungibleAsset>` argument is required.
/// - **Queries** can never be `payable`; if someone writes `#[query(payable)]`, we error out.
#[proc_macro_attribute]
pub fn account_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let account_ident = parse_macro_input!(attr as Ident);
    let mut module = parse_macro_input!(item as ItemMod);

    // Extract the "inline" contents of the module
    let content = match get_module_content(&mut module) {
        Ok(content) => content,
        Err(err) => return err.to_compile_error().into(),
    };

    // Collect the annotated functions
    let (maybe_impl_info, trait_info) = match collect_annotated_items(&account_ident, content) {
        Ok(val) => val,
        Err(err) => return err.to_compile_error().into(),
    };

    // Merge sets of init/exec/query
    let init_fn = merge_init(
        &maybe_impl_info.as_ref().and_then(|f| f.init_fn.clone()),
        &trait_info.init_fn,
    );
    if let Err(e) = init_fn {
        return e.to_compile_error().into();
    }
    let init_fn = init_fn.unwrap();

    let exec_fns = match merge_functions(
        maybe_impl_info
            .as_ref()
            .map(|f| &f.exec_fns[..])
            .unwrap_or(&[]),
        &trait_info.exec_fns,
    ) {
        Ok(e) => e,
        Err(e) => return e.to_compile_error().into(),
    };
    let query_fns = match merge_functions(
        maybe_impl_info
            .as_ref()
            .map(|f| &f.query_fns[..])
            .unwrap_or(&[]),
        &trait_info.query_fns,
    ) {
        Ok(q) => q,
        Err(e) => return e.to_compile_error().into(),
    };

    // 1) Generate message structs for all discovered functions
    let mut generated_msgs = Vec::new();
    if let Some(ref info) = init_fn {
        generated_msgs.push(generate_msg_struct(info));
    }
    for info in &exec_fns {
        generated_msgs.push(generate_msg_struct(info));
    }
    for info in &query_fns {
        generated_msgs.push(generate_msg_struct(info));
    }

    // 2) Generate the `impl AccountCode` if we have an impl for that type
    let accountcode_impl = if let Some(ref impl_info) = maybe_impl_info {
        generate_accountcode_impl(
            &account_ident,
            &impl_info.init_fn,
            &impl_info.exec_fns,
            &impl_info.query_fns,
        )
    } else {
        quote! {}
    };

    // 3) Generate the "wrapper account" struct + impl
    let wrapper_struct = generate_wrapper_struct(&account_ident, &init_fn, &exec_fns, &query_fns);

    // Combine it all
    let file: syn::File = match syn::parse2(quote! {
        #(#generated_msgs)*
        #accountcode_impl
        #wrapper_struct
    }) {
        Ok(file) => file,
        Err(e) => return e.to_compile_error().into(),
    };

    // Append
    content.extend(file.items);
    TokenStream::from(quote! { #module })
}

// -----------------------------------------
//  Data Structures
// -----------------------------------------

/// Which kind of annotated function we found, with a payable flag for init/exec.
#[derive(Clone, PartialEq)]
enum FunctionKind {
    Init { payable: bool },
    Exec { payable: bool },
    Query,
}

/// Stores details about each annotated function.
#[derive(Clone)]
struct FunctionInfo {
    fn_name: Ident,
    msg_name: Ident,
    kind: FunctionKind,
    params: Vec<(Ident, Type)>,
    return_type: Type, // the T in `-> SdkResult<T>`
}

/// Holds the aggregated info (init/exec/query) from a particular impl or trait.
struct CollectedInfo {
    init_fn: Option<FunctionInfo>,
    exec_fns: Vec<FunctionInfo>,
    query_fns: Vec<FunctionInfo>,
}

// -----------------------------------------
//  Top-level gather
// -----------------------------------------

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

/// Collect annotated methods from `impl SomeType`.
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

/// Collect annotated methods from a `trait SomeIdent`.
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

/// Insert a newly found annotated function into the right place.
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

/// Merges all (init/exec/query) from `source` into `target`.
fn merge_collected_info(
    target: &mut CollectedInfo,
    source: CollectedInfo,
) -> Result<(), syn::Error> {
    // Merge init
    if target.init_fn.is_some() && source.init_fn.is_some() {
        return Err(syn::Error::new(
            source.init_fn.unwrap().fn_name.span(),
            "Multiple #[init] found.",
        ));
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

/// Extract a `FunctionInfo` if the signature has `#[init(...)]`, `#[exec(...)]`, or `#[query(...)]`.
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

/// Parse `-> SdkResult<T>` to extract `T`.
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

/// Returns `Some(FunctionKind)` if we see `#[init]`, `#[init(payable)]`, etc.
/// Otherwise `None`.
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

/// If there's no parentheses => returns false (non-payable).
/// If parentheses exist, parse them and return true only if they're exactly `payable`.
fn parse_is_payable(attr: &Attribute) -> Result<bool, syn::Error> {
    let meta = &attr.meta; // parse into a Meta

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

fn is_impl_for_account(account_ident: &Ident, impl_block: &ItemImpl) -> bool {
    if let Type::Path(tp) = &*impl_block.self_ty {
        if let Some(seg) = tp.path.segments.last() {
            return seg.ident == *account_ident;
        }
    }
    false
}

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
//  Merging logic
// -----------------------------------------

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

fn generate_msg_struct(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let msg_name = &info.msg_name;
    let fn_name_str = info.fn_name.to_string();
    let fields = info.params.iter().map(|(ident, ty)| {
        quote! { pub #ident: #ty, }
    });

    // Generate an 8-byte ID from the function name
    let mut hasher = Sha256::new();
    hasher.update(fn_name_str.as_bytes());
    let hash = hasher.finalize();
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&hash[..8]);
    let fn_id = u64::from_le_bytes(arr);

    match info.kind {
        FunctionKind::Init { .. } => quote! {
            #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize)]
            pub struct #msg_name {
                #(#fields)*
            }
        },
        FunctionKind::Exec { .. } | FunctionKind::Query => {
            quote! {
                #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize)]
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

// -----------------------------------------
// 2) Generate `impl AccountCode for {account_ident}`
// -----------------------------------------

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
                    Ok(::evolve_core::InvokeResponse::new(&resp)?)
                }
            }
        });
        quote! {
            fn query(&self, env: &dyn ::evolve_core::Environment, request: &::evolve_core::InvokeRequest)
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

/// Generate the `fn init(...) { ... }` body with or without a payable check.
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
            Ok(::evolve_core::InvokeResponse::new(&resp)?)
        }
    }
}

/// Generate one `match` arm for an `exec` function, including any funds check if non-payable.
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
            Ok(::evolve_core::InvokeResponse::new(&resp)?)
        }
    }
}

// -----------------------------------------
// 3) Generate the "wrapper" struct + impl
// -----------------------------------------

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
        #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize)]
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

/// Generate the wrapper for a single `#[init]` function.
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

/// Generate the wrapper for a single `#[exec]` function.
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

/// Generate the wrapper for a single `#[query]` function.
/// (Queries are never payable, so no funds param.)
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
            env: &dyn ::evolve_core::Environment
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
// Marker attributes )
// -----------------------------------------
#[proc_macro_attribute]
pub fn init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn exec(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn query(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
