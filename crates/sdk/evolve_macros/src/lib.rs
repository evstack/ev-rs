extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use sha2::{Digest, Sha256};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, FnArg, Ident, ImplItem, Item, ItemImpl,
    ItemMod, Pat, ReturnType, Signature, Type, TypePath
};
use heck::ToUpperCamelCase;

/// This attribute macro generates:
/// 1) Message types (e.g. `InitializeMsg`, `TransferMsg`, etc.).
/// 2) An `impl AccountCode` for the target struct (e.g. `Asset`).
/// 3) A "wrapper account" struct (e.g. `AssetAccount`) that has convenience methods
///    calling `create_account`, `exec_account`, and `query_account`.
#[proc_macro_attribute]
pub fn account_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let account_ident = parse_macro_input!(attr as Ident);
    let mut module = parse_macro_input!(item as ItemMod);

    let content = match get_module_content(&mut module) {
        Ok(content) => content,
        Err(err) => return err.to_compile_error().into(),
    };

    let (init_fn, exec_fns, query_fns) = match collect_account_functions(&account_ident, content) {
        Ok(val) => val,
        Err(err) => return err.to_compile_error().into(),
    };

    // 1) Generate the message struct definitions for each function.
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

    // 2) Generate the AccountCode trait impl for the "real" struct (e.g. `impl AccountCode for Asset`).
    let accountcode_impl = generate_accountcode_impl(&account_ident, &init_fn, &exec_fns, &query_fns);

    // 3) (NEW) Generate the "wrapper account" struct + impl, e.g. `pub struct AssetAccount { ... }`.
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

    // Append all the generated items to the module
    content.extend(file.items);

    TokenStream::from(quote! { #module })
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

// A small struct for capturing the annotation kind
#[derive(Clone, PartialEq)]
enum FunctionKind {
    Init,
    Exec,
    Query,
}

// Stores details about each annotated function
#[derive(Clone)]
struct FunctionInfo {
    fn_name: Ident,
    msg_name: Ident,
    kind: FunctionKind,
    params: Vec<(Ident, Type)>,
    // NEW: store the inner return type T from `-> SdkResult<T>`
    return_type: Type,
}

/// Collects the #[init], #[exec], and #[query] functions from `impl {account_ident}`.
fn collect_account_functions(
    account_ident: &Ident,
    items: &Vec<Item>,
) -> Result<(Option<FunctionInfo>, Vec<FunctionInfo>, Vec<FunctionInfo>), syn::Error> {
    let mut init_fn: Option<FunctionInfo> = None;
    let mut exec_fns = Vec::new();
    let mut query_fns = Vec::new();

    for item in items {
        let impl_block = match item {
            Item::Impl(imp) => imp,
            _ => continue,
        };
        if !is_impl_for_account(account_ident, impl_block) {
            continue;
        }

        for impl_item in &impl_block.items {
            let method = match impl_item {
                ImplItem::Fn(m) => m,
                _ => continue,
            };
            let marker = match get_function_marker(method) {
                Some(m) => m,
                None => continue,
            };

            // Derive the name for the generated message struct.
            let fn_name = method.sig.ident.clone();
            let msg_name = format_ident!("{}Msg", fn_name.to_string().to_upper_camel_case());

            // Basic param checks:
            let inputs: Vec<_> = method.sig.inputs.iter().collect();
            if inputs.len() < 2 {
                return Err(syn::Error::new(
                    method.sig.ident.span(),
                    "Expected at least two parameters: a receiver and an environment",
                ));
            }
            if !matches!(inputs[0], FnArg::Receiver(_)) {
                return Err(syn::Error::new(
                    inputs[0].span(),
                    "Expected first param to be &self or &mut self",
                ));
            }
            let env_input = inputs.last().unwrap();
            // If it's a query function, env must be &dyn ...
            // Otherwise &mut dyn ...
            // (We already do that logic in your original macro.)

            // Gather the "middle" params as message fields
            let field_inputs = &inputs[1..inputs.len()-1];
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
                        // Should never happen in the "middle"
                        return Err(syn::Error::new(
                            input.span(),
                            "Unexpected receiver in function parameters",
                        ));
                    }
                }
            }

            // NEW: parse out the function's return type T from `-> SdkResult<T>`
            let return_type = parse_return_type(&method.sig)?;

            let info = FunctionInfo {
                fn_name,
                msg_name,
                kind: marker.clone(),
                params,
                return_type,
            };
            match marker {
                FunctionKind::Init => {
                    if init_fn.is_some() {
                        return Err(syn::Error::new(
                            method.sig.ident.span(),
                            "Multiple #[init] functions are not allowed.",
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

/// Looks at the function signature and extracts the inner type from `-> SdkResult<T>`.
fn parse_return_type(sig: &Signature) -> Result<Type, syn::Error> {
    match &sig.output {
        ReturnType::Default => {
            // i.e. no "-> something"
            // If you require always returning SdkResult<T>, throw error here.
            // Or just treat it as "()".
            let unit: Type = syn::parse_quote! { () };
            Ok(unit)
        }
        ReturnType::Type(_, ty) => {
            // We expect: -> SdkResult<...>
            // Let’s do a naive check
            if let Type::Path(TypePath { path, .. }) = &**ty {
                let last = path.segments.last();
                if let Some(seg) = last {
                    if seg.ident == "SdkResult" {
                        // If it’s exactly SdkResult<T>:
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

fn is_marker_attr(attr: &Attribute, marker: &str) -> bool {
    attr.path()
        .get_ident()
        .map_or(false, |ident| ident == marker)
}

/// Returns which of #[init], #[exec], #[query] is used, if any.
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

/// Verifies that `impl_block` is exactly `impl {account_ident} { ... }`.
fn is_impl_for_account(account_ident: &Ident, impl_block: &ItemImpl) -> bool {
    if let Type::Path(tp) = &*impl_block.self_ty {
        if let Some(seg) = tp.path.segments.last() {
            return seg.ident == *account_ident;
        }
    }
    false
}

// ------------------------------------------------------
// 1) Generate each message struct
// ------------------------------------------------------
fn generate_msg_struct(info: &FunctionInfo) -> proc_macro2::TokenStream {
    let msg_name = &info.msg_name;
    let fn_name_str = info.fn_name.to_string();
    let fields = info.params.iter().map(|(ident, ty)| {
        quote! { pub #ident: #ty, }
    });

    // Generate an 8-byte ID from the function name (like you had before)
    let mut hasher = Sha256::new();
    hasher.update(fn_name_str.as_bytes());
    let hash = hasher.finalize();
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&hash[..8]);
    let fn_id = u64::from_le_bytes(arr);

    match info.kind {
        FunctionKind::Init => quote! {
            #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize)]
            pub struct #msg_name {
                #(#fields)*
            }
        },
        FunctionKind::Exec | FunctionKind::Query => {
            quote! {
                #[derive(::borsh::BorshSerialize, ::borsh::BorshDeserialize)]
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
}

// ------------------------------------------------------
// 2) Generate `impl AccountCode for {account_ident}`
// ------------------------------------------------------
fn generate_accountcode_impl(
    account_ident: &Ident,
    init_fn: &Option<FunctionInfo>,
    exec_fns: &[FunctionInfo],
    query_fns: &[FunctionInfo],
) -> proc_macro2::TokenStream {
    let account_name_str = account_ident.to_string();

    // init branch
    let init_impl = if let Some(info) = init_fn {
        let msg_name = &info.msg_name;
        let fn_name = &info.fn_name;
        let args = info.params.iter().map(|(n, _)| quote!{ msg.#n });
        quote! {
            fn init(&self, env: &mut dyn ::evolve_core::Environment, request: ::evolve_core::InvokeRequest)
                -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
            {
                use evolve_core::encoding::{Decodable, Encodable};
                let msg = request.decode::<#msg_name>()?;
                let resp = self.#fn_name(#(#args, )* env)?;
                let encoded = resp.encode()?;
                let msg_resp = ::evolve_core::Message::from(encoded);
                Ok(::evolve_core::InvokeResponse::new(msg_resp))
            }
        }
    } else {
        quote! {
            fn init(&self, _env: &mut dyn ::evolve_core::Environment, _request: ::evolve_core::InvokeRequest)
                -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
            {
                Err(::evolve_core::ERR_UNKNOWN_FUNCTION)
            }
        }
    };

    // exec branch
    let exec_arms = exec_fns.iter().map(|info| {
        let fn_id = format_ident!("FUNCTION_IDENTIFIER");
        let msg_name = &info.msg_name;
        let fn_name = &info.fn_name;
        let args = info.params.iter().map(|(n, _)| quote!( msg.#n ));
        quote! {
            #msg_name::#fn_id => {
                let msg = request.decode::<#msg_name>()?;
                let resp = self.#fn_name(#(#args, )* env)?;
                ::evolve_core::InvokeResponse::try_from_encodable(resp)
            }
        }
    });
    let exec_impl = quote! {
        fn execute(&self, env: &mut dyn ::evolve_core::Environment, request: ::evolve_core::InvokeRequest)
            -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
        {
            use evolve_core::encoding::Decodable;
            match request.function() {
                #(#exec_arms,)*
                _ => Err(::evolve_core::ERR_UNKNOWN_FUNCTION)
            }
        }
    };

    // query branch
    let query_arms = query_fns.iter().map(|info| {
        let fn_id = format_ident!("FUNCTION_IDENTIFIER");
        let msg_name = &info.msg_name;
        let fn_name = &info.fn_name;
        let args = info.params.iter().map(|(n, _)| quote!( msg.#n ));
        quote! {
            #msg_name::#fn_id => {
                let msg = request.decode::<#msg_name>()?;
                let resp = self.#fn_name(#(#args, )* env)?;
                ::evolve_core::InvokeResponse::try_from_encodable(resp)
            }
        }
    });
    let query_impl = quote! {
        fn query(&self, env: &dyn ::evolve_core::Environment, request: ::evolve_core::InvokeRequest)
            -> ::evolve_core::SdkResult<::evolve_core::InvokeResponse>
        {
            use evolve_core::encoding::Decodable;
            match request.function() {
                #(#query_arms,)*
                _ => Err(::evolve_core::ERR_UNKNOWN_FUNCTION)
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

// ------------------------------------------------------
// 3) (NEW) Generate the "wrapper" struct + impl
// ------------------------------------------------------
fn generate_wrapper_struct(
    account_ident: &Ident,
    init_fn: &Option<FunctionInfo>,
    exec_fns: &[FunctionInfo],
    query_fns: &[FunctionInfo],
) -> proc_macro2::TokenStream {
    // We'll call it e.g. `AssetAccount` if the user typed `#[account_impl(Asset)]`
    let wrapper_ident = format_ident!("{}Account", account_ident);

    // Generate wrapper methods for the init function (if any).
    let init_method = init_fn.as_ref().map(|info| {
        let fn_name = &info.fn_name; // e.g. "initialize"
        let msg_name = &info.msg_name; // e.g. "InitializeMsg"
        let return_inner = &info.return_type; // e.g. Option<u128> or ()
        // In practice, your user-defined init always returns SdkResult<()>, but let's be general.

        // Expand the param list for the wrapper method (excluding env).
        // We also remember to add `env: &mut dyn Environment` at the end.
        // The user’s function param list is in `info.params`.
        let params_decl = info.params.iter().map(|(n, t)| {
            quote!{ #n: #t }
        });
        let param_names = info.params.iter().map(|(n, _)| quote!(#n));

        // The wrapper method calls `create_account(..., &<Msg>{...}, env)`,
        // then stores the newly created account ID in self.0 (which is an Item<AccountId>).
        quote! {
            pub fn #fn_name(&self, #( #params_decl, )* env: &mut dyn ::evolve_core::Environment)
                -> ::evolve_core::SdkResult<#return_inner>
            {
                let (acc_id, resp) = ::evolve_core::low_level::create_account(
                    // The real contract name as a string
                    stringify!(#account_ident).to_string(),
                    &#msg_name { #( #param_names, )* },
                    env,
                )?;
                // store the created account ID in our Item
                self.0.set(&acc_id, env)?;
                Ok(resp)
            }
        }
    });

    // Generate wrapper methods for each exec
    let exec_methods = exec_fns.iter().map(|info| {
        let fn_name = &info.fn_name;
        let msg_name = &info.msg_name;
        let return_inner = &info.return_type;

        let params_decl = info.params.iter().map(|(n, t)| quote!{ #n: #t });
        let param_names = info.params.iter().map(|(n, _)| quote!(#n));

        quote! {
            pub fn #fn_name(&self, #( #params_decl, )* env: &mut dyn ::evolve_core::Environment)
                -> ::evolve_core::SdkResult<#return_inner>
            {
                ::evolve_core::low_level::exec_account(
                    self.0.get(env)?.ok_or(::evolve_core::ERR_ACCOUNT_NOT_INITIALIZED)?,
                    #msg_name::FUNCTION_IDENTIFIER,
                    &#msg_name { #( #param_names, )* },
                    env,
                )
            }
        }
    });

    // Generate wrapper methods for each query
    let query_methods = query_fns.iter().map(|info| {
        let fn_name = &info.fn_name;
        let msg_name = &info.msg_name;
        let return_inner = &info.return_type;

        let params_decl = info.params.iter().map(|(n, t)| quote!{ #n: #t });
        let param_names = info.params.iter().map(|(n, _)| quote!(#n));

        quote! {
            pub fn #fn_name(&self, #( #params_decl, )* env: &dyn ::evolve_core::Environment)
                -> ::evolve_core::SdkResult<#return_inner>
            {
                ::evolve_core::low_level::query_account(
                    self.0.get(env)?.ok_or(::evolve_core::ERR_ACCOUNT_NOT_INITIALIZED)?,
                    #msg_name::FUNCTION_IDENTIFIER,
                    &#msg_name { #( #param_names, )* },
                    env,
                )
            }
        }
    });

    quote! {
        /// A generated "wrapper" struct that holds the account-id pointer
        /// and provides convenience methods for init/exec/query calls.
        pub struct #wrapper_ident(pub ::evolve_collections::Item<::evolve_core::AccountId>);

        impl #wrapper_ident {
            /// Create the item pointer with a user-supplied prefix
            pub fn new(prefix: u8) -> Self {
                Self(::evolve_collections::Item::new(prefix))
            }

            // Optionally a helper to set the pointer manually
            pub fn set_account_id_ptr(
                &self,
                account_id: ::evolve_core::AccountId,
                env: &mut dyn ::evolve_core::Environment
            ) -> ::evolve_core::SdkResult<()> {
                self.0.set(&account_id, env)
            }

            #init_method
            #( #exec_methods )*
            #( #query_methods )*
        }
    }
}

// -------------------------
//  Marker attributes
// -------------------------
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
