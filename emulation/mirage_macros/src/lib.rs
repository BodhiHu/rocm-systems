use std::collections::BTreeMap;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{
    Attribute, Error, Ident, LitStr, Path, Result, Token, Type, Visibility, braced, parenthesized,
    parse_macro_input,
};

#[proc_macro]
pub fn ctl_dsl(item: TokenStream) -> TokenStream {
    let module = parse_macro_input!(item as CtlModule);
    match expand_ctl_module(module) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

struct CtlModule {
    attrs: Vec<Attribute>,
    vis: Visibility,
    ident: Ident,
    endpoints: Vec<Endpoint>,
}

impl Parse for CtlModule {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        input.parse::<Token![mod]>()?;
        let ident = input.parse()?;

        let content;
        braced!(content in input);

        let mut endpoints = Vec::new();
        while !content.is_empty() {
            endpoints.push(content.parse()?);
        }

        Ok(Self {
            attrs,
            vis,
            ident,
            endpoints,
        })
    }
}

struct Endpoint {
    attrs: Vec<Attribute>,
    ident: Ident,
    request: FieldBlock,
    input: Option<FieldBlock>,
    output: FieldBlock,
    reply: Option<FieldBlock>,
    errors: Vec<Path>,
}

impl Parse for Endpoint {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let ident = input.parse()?;

        let request_content;
        parenthesized!(request_content in input);
        let request = request_content.parse()?;

        let input_stream = if peek_ellipsis(input) {
            input.parse::<Token![.]>()?;
            input.parse::<Token![.]>()?;
            input.parse::<Token![.]>()?;
            Some(input.parse()?)
        } else {
            None
        };

        input.parse::<Token![->]>()?;
        let output = input.parse()?;

        let reply = if input.peek(Token![=>]) {
            input.parse::<Token![=>]>()?;
            Some(input.parse()?)
        } else {
            None
        };

        let errors = if input.peek(Token![?]) {
            input.parse::<Token![?]>()?;
            let mut errors = Vec::new();
            loop {
                errors.push(input.parse()?);
                if !input.peek(Token![,]) {
                    break;
                }
                input.parse::<Token![,]>()?;
            }
            errors
        } else {
            Vec::new()
        };

        input.parse::<Token![;]>()?;

        Ok(Self {
            attrs,
            ident,
            request,
            input: input_stream,
            output,
            reply,
            errors,
        })
    }
}

fn peek_ellipsis(input: ParseStream<'_>) -> bool {
    input.peek(Token![.]) && input.peek2(Token![.]) && input.peek3(Token![.])
}

struct FieldBlock {
    fields: Vec<FieldSpec>,
}

impl Parse for FieldBlock {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let content;
        braced!(content in input);

        let mut fields = Vec::new();
        while !content.is_empty() {
            fields.push(content.parse()?);
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }

        Ok(Self { fields })
    }
}

struct FieldSpec {
    attrs: Vec<Attribute>,
    ident: Ident,
    ty: Type,
    optional: bool,
}

impl Parse for FieldSpec {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty = input.parse()?;

        let optional = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let none_ident: Ident = input.parse()?;
            if none_ident != "None" {
                return Err(Error::new_spanned(
                    none_ident,
                    "ctl_dsl only supports `= None` for optional fields",
                ));
            }
            true
        } else {
            false
        };

        Ok(Self {
            attrs,
            ident,
            ty,
            optional,
        })
    }
}

#[derive(Default)]
struct ModuleOptions {
    shared_error: Option<Path>,
    protocol_error: Option<Path>,
    cli_name: Option<String>,
}

#[derive(Default)]
struct EndpointOptions {
    skip_cli: bool,
}

fn split_module_attrs(attrs: Vec<Attribute>) -> Result<(Vec<Attribute>, ModuleOptions)> {
    let mut filtered = Vec::new();
    let mut options = ModuleOptions::default();

    for attr in attrs {
        if !attr.path().is_ident("ctl") {
            filtered.push(attr);
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("error") {
                if options.shared_error.is_some() {
                    return Err(meta.error("duplicate `error` option"));
                }
                options.shared_error = Some(meta.value()?.parse()?);
                return Ok(());
            }

            if meta.path.is_ident("protocol_error") {
                if options.protocol_error.is_some() {
                    return Err(meta.error("duplicate `protocol_error` option"));
                }
                options.protocol_error = Some(meta.value()?.parse()?);
                return Ok(());
            }

            if meta.path.is_ident("cli_name") {
                if options.cli_name.is_some() {
                    return Err(meta.error("duplicate `cli_name` option"));
                }
                options.cli_name = Some(meta.value()?.parse::<LitStr>()?.value());
                return Ok(());
            }

            Err(meta.error("unsupported ctl module option"))
        })?;
    }

    Ok((filtered, options))
}

fn split_endpoint_attrs(attrs: &[Attribute]) -> Result<(Vec<Attribute>, EndpointOptions)> {
    let mut filtered = Vec::new();
    let mut options = EndpointOptions::default();

    for attr in attrs {
        if !attr.path().is_ident("ctl") {
            filtered.push(attr.clone());
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip_cli") {
                options.skip_cli = true;
                return Ok(());
            }

            Err(meta.error("unsupported ctl endpoint option"))
        })?;
    }

    Ok((filtered, options))
}

fn expand_ctl_module(module: CtlModule) -> Result<TokenStream2> {
    let CtlModule {
        attrs,
        vis,
        ident,
        endpoints,
    } = module;
    let (attrs, module_options) = split_module_attrs(attrs)?;

    if module_options.shared_error.is_some() != module_options.protocol_error.is_some() {
        return Err(Error::new_spanned(
            &ident,
            "ctl modules must configure both `error = ...` and `protocol_error = ...`, or neither",
        ));
    }

    let module_prefix = pascal_case_ident(&ident);
    let cli_name = format_ident!("{module_prefix}Cli");
    let command_name = format_ident!("{module_prefix}Command");
    let request_enum = format_ident!("{module_prefix}Request");
    let input_enum = format_ident!("{module_prefix}Input");
    let output_enum = format_ident!("{module_prefix}Output");
    let reply_enum = format_ident!("{module_prefix}Reply");
    let error_enum = format_ident!("{module_prefix}Error");
    let call_enum = format_ident!("{module_prefix}Call");
    let impl_trait = format_ident!("{module_prefix}Impl");
    let transport_trait = format_ident!("{module_prefix}Transport");
    let transport_dispatch_fn = format_ident!("dispatch_transport");
    let cli_binary_name = module_options
        .cli_name
        .clone()
        .unwrap_or_else(|| ident.to_string().replace('_', "-"));
    let module_error_ty = module_error_type_tokens(&module_options, &error_enum);

    let mut generated_types = Vec::new();
    let mut error_defs: BTreeMap<String, TokenStream2> = BTreeMap::new();
    let mut trait_defs = Vec::new();
    let mut transport_impls = Vec::new();
    let mut command_args = Vec::new();
    let mut command_variants = Vec::new();
    let mut request_variants = Vec::new();
    let mut input_variants = Vec::new();
    let mut output_variants = Vec::new();
    let mut reply_variants = Vec::new();
    let mut error_variants = Vec::new();
    let mut call_variants = Vec::new();
    let mut dispatch_arms = Vec::new();
    let mut transport_dispatch_arms = Vec::new();
    let mut impl_supertraits = Vec::new();
    let mut run_arms = Vec::new();
    let mut print_output_arms = Vec::new();
    let mut print_reply_arms = Vec::new();
    let mut request_kind_arms = Vec::new();
    let mut input_kind_arms = Vec::new();
    let mut output_kind_arms = Vec::new();
    let mut reply_kind_arms = Vec::new();

    for endpoint in &endpoints {
        let (endpoint_attrs, endpoint_options) = split_endpoint_attrs(&endpoint.attrs)?;
        let endpoint_prefix = pascal_case_ident(&endpoint.ident);
        let request_name = format_ident!("{endpoint_prefix}Request");
        let output_name = format_ident!("{endpoint_prefix}Output");
        let reply_name = format_ident!("{endpoint_prefix}Reply");
        let error_name = format_ident!("{endpoint_prefix}Error");
        let impl_name = format_ident!("Impl{endpoint_prefix}");
        let command_args_name = format_ident!("{endpoint_prefix}CommandArgs");
        let variant_name = endpoint_prefix.clone();
        let endpoint_error_ty = endpoint_error_type_tokens(&module_options, &error_name);
        let input_name = endpoint
            .input
            .as_ref()
            .map(|_| format_ident!("{endpoint_prefix}Input"));

        if module_options.shared_error.is_some() && !endpoint.errors.is_empty() {
            return Err(Error::new_spanned(
                &endpoint.ident,
                "endpoint-specific errors are not supported when module-level `error` is configured",
            ));
        }

        generated_types.push(expand_struct(&endpoint_attrs, &request_name, &endpoint.request));
        generated_types.push(expand_struct(&endpoint_attrs, &output_name, &endpoint.output));

        if let Some(input_block) = &endpoint.input {
            generated_types.push(expand_struct(
                &endpoint_attrs,
                input_name.as_ref().expect("input name must exist"),
                input_block,
            ));
        }

        if let Some(reply_block) = &endpoint.reply {
            generated_types.push(expand_struct(&endpoint_attrs, &reply_name, reply_block));
        } else {
            generated_types.push(quote! {
                #( #endpoint_attrs )*
                pub type #reply_name = #output_name;
            });
        }

        if let Some(shared_error) = &module_options.shared_error {
            generated_types.push(quote! {
                pub type #error_name = #shared_error;
            });
        } else {
            for error_path in &endpoint.errors {
                if let Some(error_ident) = error_path.get_ident() {
                    error_defs.entry(error_ident.to_string()).or_insert_with(|| {
                        let message = humanize_ident(error_ident);
                        quote! {
                            #[derive(
                                Debug,
                                Clone,
                                PartialEq,
                                Eq,
                                ::serde::Serialize,
                                ::serde::Deserialize,
                                ::thiserror::Error,
                            )]
                            #[error(#message)]
                            pub struct #error_ident;
                        }
                    });
                }
            }

            let endpoint_error_variants = endpoint.errors.iter().map(|error_path| {
                let variant_ident = last_path_ident(error_path);
                quote! {
                    #[error(transparent)]
                    #variant_ident(#[from] #error_path),
                }
            });

            generated_types.push(quote! {
                #[derive(
                    Debug,
                    Clone,
                    PartialEq,
                    Eq,
                    ::serde::Serialize,
                    ::serde::Deserialize,
                    ::thiserror::Error,
                )]
                pub enum #error_name {
                    #( #endpoint_error_variants )*
                    #[error("{0}")]
                    Other(String),
                }
            });
        }

        trait_defs.push(expand_impl_trait(
            &endpoint_attrs,
            &impl_name,
            &endpoint.ident,
            &request_name,
            input_name.as_ref(),
            &output_name,
            &reply_name,
            &endpoint_error_ty,
            endpoint.reply.is_some(),
        ));
        impl_supertraits.push(quote! { #impl_name });

        request_variants.push(quote! { #variant_name(#request_name), });
        request_kind_arms.push(quote! {
            Self::#variant_name(_) => stringify!(#variant_name),
        });
        output_variants.push(quote! { #variant_name(#output_name), });
        output_kind_arms.push(quote! {
            Self::#variant_name(_) => stringify!(#variant_name),
        });
        reply_variants.push(quote! { #variant_name(#reply_name), });
        reply_kind_arms.push(quote! {
            Self::#variant_name(_) => stringify!(#variant_name),
        });
        if module_options.shared_error.is_none() {
            error_variants.push(quote! {
                #[error(transparent)]
                #variant_name(#[from] #error_name),
            });
        }

        if let Some(input_name) = &input_name {
            input_variants.push(quote! { #variant_name(#input_name), });
            input_kind_arms.push(quote! {
                Self::#variant_name(_) => stringify!(#variant_name),
            });
        }

        if endpoint.reply.is_some() {
            let dispatch_map_err = if module_options.shared_error.is_some() {
                TokenStream2::new()
            } else {
                quote! { .map_err(#error_enum::from) }
            };
            let input_tokens = if let Some(input_name) = &input_name {
                quote! { input: ::tokio::sync::mpsc::Receiver<#input_name>, }
            } else {
                TokenStream2::new()
            };

            call_variants.push(quote! {
                #variant_name {
                    request: #request_name,
                    #input_tokens
                    output: ::tokio::sync::mpsc::Sender<#output_name>,
                },
            });

            let dispatch_inputs = if input_name.is_some() {
                quote! { input, }
            } else {
                TokenStream2::new()
            };
            let method = &endpoint.ident;
            dispatch_arms.push(quote! {
                #call_enum::#variant_name {
                    request,
                    #dispatch_inputs
                    output,
                } => {
                    self.#method(request, #dispatch_inputs output)
                        .await
                        .map(#reply_enum::#variant_name)
                        #dispatch_map_err
                }
            });

            transport_impls.push(expand_transport_impl(
                &module_options,
                &transport_trait,
                &impl_name,
                &endpoint.ident,
                &request_name,
                input_name.as_ref(),
                &output_name,
                &reply_name,
                &error_name,
                &request_enum,
                &input_enum,
                &output_enum,
                &reply_enum,
                &variant_name,
                true,
            ));
            transport_dispatch_arms.push(expand_transport_dispatch_arm(
                &module_options,
                &variant_name,
                &endpoint.ident,
                &request_enum,
                &input_enum,
                &output_enum,
                &reply_enum,
                &error_enum,
                &request_name,
                input_name.as_ref(),
                &output_name,
                &reply_name,
                true,
            ));
            if !endpoint_options.skip_cli {
                run_arms.push(expand_streaming_run_arm(
                    &command_name,
                    &variant_name,
                    &request_name,
                    &call_enum,
                    input_name.as_ref(),
                    &output_name,
                    &output_enum,
                    endpoint,
                ));
            }
            print_output_arms.push(expand_print_output_arm(
                &output_enum,
                &variant_name,
                endpoint,
            ));
        } else {
            call_variants.push(quote! { #variant_name(#request_name), });
            let method = &endpoint.ident;
            let dispatch_map_err = if module_options.shared_error.is_some() {
                TokenStream2::new()
            } else {
                quote! { .map_err(#error_enum::from) }
            };
            dispatch_arms.push(quote! {
                #call_enum::#variant_name(request) => {
                    self.#method(request)
                        .await
                        .map(#reply_enum::#variant_name)
                        #dispatch_map_err
                }
            });

            transport_impls.push(expand_transport_impl(
                &module_options,
                &transport_trait,
                &impl_name,
                &endpoint.ident,
                &request_name,
                None,
                &output_name,
                &reply_name,
                &error_name,
                &request_enum,
                &input_enum,
                &output_enum,
                &reply_enum,
                &variant_name,
                false,
            ));
            transport_dispatch_arms.push(expand_transport_dispatch_arm(
                &module_options,
                &variant_name,
                &endpoint.ident,
                &request_enum,
                &input_enum,
                &output_enum,
                &reply_enum,
                &error_enum,
                &request_name,
                None,
                &output_name,
                &reply_name,
                false,
            ));
            if !endpoint_options.skip_cli {
                run_arms.push(expand_unary_run_arm(
                    &command_name,
                    &variant_name,
                    &request_name,
                    &call_enum,
                    &endpoint.request,
                ));
            }
        }

        print_reply_arms.push(expand_print_reply_arm(&reply_enum, &variant_name, endpoint));

        if !endpoint_options.skip_cli {
            let cli_fields = endpoint.request.fields.iter().map(expand_cli_field);
            command_args.push(quote! {
                #( #endpoint_attrs )*
                #[derive(Debug, Clone, ::clap::Args)]
                pub struct #command_args_name {
                    #( #cli_fields )*
                }
            });
            command_variants.push(quote! {
                #( #endpoint_attrs )*
                #variant_name(#command_args_name),
            });
        }
    }

    let error_defs = error_defs.values().collect::<Vec<_>>();
    let error_def = if let Some(shared_error) = &module_options.shared_error {
        quote! {
            pub type #error_enum = #shared_error;
        }
    } else {
        quote! {
            #[derive(
                Debug,
                Clone,
                ::serde::Serialize,
                ::serde::Deserialize,
                ::thiserror::Error,
            )]
            pub enum #error_enum {
                #( #error_variants )*
                #[error("{0}")]
                Other(String),
            }
        }
    };

    Ok(quote! {
        #( #attrs )*
        #vis mod #ident {
            #[allow(unused_imports)]
            use super::*;

            #( #generated_types )*
            #( #error_defs )*
            #( #trait_defs )*
            #( #transport_impls )*
            #( #command_args )*

            /// All control-protocol request payloads keyed by endpoint.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #request_enum {
                #( #request_variants )*
            }

            impl #request_enum {
                /// Returns the endpoint name associated with this request payload.
                pub fn kind(&self) -> &'static str {
                    match self {
                        #( #request_kind_arms )*
                    }
                }
            }

            /// All streaming input payloads keyed by endpoint.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #input_enum {
                #( #input_variants )*
            }

            impl #input_enum {
                /// Returns the endpoint name associated with this streaming input payload.
                pub fn kind(&self) -> &'static str {
                    match self {
                        #( #input_kind_arms )*
                    }
                }
            }

            /// All streaming output payloads keyed by endpoint.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #output_enum {
                #( #output_variants )*
            }

            impl #output_enum {
                /// Returns the endpoint name associated with this streaming output payload.
                pub fn kind(&self) -> &'static str {
                    match self {
                        #( #output_kind_arms )*
                    }
                }
            }

            /// All control-protocol reply payloads keyed by endpoint.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #reply_enum {
                #( #reply_variants )*
            }

            impl #reply_enum {
                /// Returns the endpoint name associated with this reply payload.
                pub fn kind(&self) -> &'static str {
                    match self {
                        #( #reply_kind_arms )*
                    }
                }
            }

            /// Error type returned while serving or transporting control-protocol calls.
            #error_def

            /// Internal call representation used by generated local dispatch helpers.
            pub enum #call_enum {
                #( #call_variants )*
            }

            #[::async_trait::async_trait]
            pub trait #impl_trait: #( #impl_supertraits + )* Send + Sync {
                /// Dispatches a generated control call to the matching endpoint method.
                async fn dispatch(&self, call: #call_enum) -> Result<#reply_enum, #module_error_ty> {
                    match call {
                        #( #dispatch_arms, )*
                    }
                }
            }

            impl<T> #impl_trait for T where T: #( #impl_supertraits + )* Send + Sync {}

            /// Transport abstraction that can carry generated control-protocol requests,
            /// optional streaming inputs, streaming outputs, and final replies.
            #[::async_trait::async_trait]
            pub trait #transport_trait: Send + Sync {
                /// Sends one generated control request over a transport and waits for
                /// the matching reply while forwarding any streaming traffic.
                async fn transport_call(
                    &self,
                    request: #request_enum,
                    input: ::std::option::Option<::tokio::sync::mpsc::Receiver<#input_enum>>,
                    output: ::std::option::Option<::tokio::sync::mpsc::Sender<#output_enum>>,
                ) -> Result<#reply_enum, #module_error_ty>;
            }

            /// Adapts generated request/input/output enums onto the endpoint-specific
            /// traits implemented by a control service.
            pub async fn #transport_dispatch_fn<T>(
                ctl: &T,
                request: #request_enum,
                input: ::std::option::Option<::tokio::sync::mpsc::Receiver<#input_enum>>,
                output: ::std::option::Option<::tokio::sync::mpsc::Sender<#output_enum>>,
            ) -> Result<#reply_enum, #module_error_ty>
            where
                T: #impl_trait + ?Sized,
            {
                match request {
                    #( #transport_dispatch_arms, )*
                }
            }

            #[derive(Debug, Clone, ::clap::Subcommand)]
            pub enum #command_name {
                #( #command_variants )*
            }

            #[derive(Debug, Clone, ::clap::Parser)]
            #[command(name = #cli_binary_name)]
            pub struct #cli_name {
                /// Path to the Unix-domain socket used for ctl RPCs.
                #[arg(long, global = true, default_value_os_t = crate::paths::socket_path())]
                pub socket: ::std::path::PathBuf,

                /// Print replies as formatted JSON instead of human-readable text.
                #[arg(long, global = true)]
                pub json: bool,

                /// The control-protocol endpoint to invoke.
                #[command(subcommand)]
                pub command: #command_name,
            }

            impl #cli_name {
                /// Executes the selected control-protocol subcommand against `ctl`.
                pub async fn run_with<T>(self, ctl: &T) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>>
                where
                    T: #impl_trait + ?Sized,
                {
                    let json = self.json;
                    match self.command {
                        #( #run_arms )*
                    }
                    Ok(())
                }

                /// Serializes and prints a value as pretty JSON.
                fn print_json<T>(value: &T) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>>
                where
                    T: ::serde::Serialize,
                {
                    println!("{}", ::serde_json::to_string_pretty(value)?);
                    Ok(())
                }

                /// Prints a streamed output item using either JSON or the generated
                /// endpoint-specific default formatter.
                async fn print_output(
                    json: bool,
                    output: &#output_enum,
                ) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    match output {
                        #( #print_output_arms )*
                        other => {
                            if json {
                                Self::print_json(other)?;
                            } else {
                                println!("{other:#?}");
                            }
                        }
                    }
                    Ok(())
                }

                /// Prints a final reply using either JSON or the generated
                /// endpoint-specific default formatter.
                async fn print_reply(
                    json: bool,
                    reply: &#reply_enum,
                ) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>> {
                    match reply {
                        #( #print_reply_arms )*
                    }
                    Ok(())
                }
            }
        }
    })
}

fn expand_struct(attrs: &[Attribute], name: &Ident, block: &FieldBlock) -> TokenStream2 {
    let default_derive = if block.fields.is_empty() || block.fields.iter().all(|field| field.optional) {
        Some(quote! { Default, })
    } else {
        None
    };
    let fields = block.fields.iter().map(|field| {
        let field_attrs = filter_clap_attrs(&field.attrs);
        let ident = &field.ident;
        let ty = if field.optional {
            let inner = &field.ty;
            quote! { ::std::option::Option<#inner> }
        } else {
            let ty = &field.ty;
            quote! { #ty }
        };

        let serde_attr = if field.optional {
            Some(quote! { #[serde(default, skip_serializing_if = "Option::is_none")] })
        } else {
            None
        };

        quote! {
            #( #field_attrs )*
            #serde_attr
            pub #ident: #ty,
        }
    });

    quote! {
        #( #attrs )*
        #[derive(Debug, Clone, #default_derive PartialEq, ::serde::Serialize, ::serde::Deserialize)]
        pub struct #name {
            #( #fields )*
        }
    }
}

fn module_error_type_tokens(options: &ModuleOptions, error_enum: &Ident) -> TokenStream2 {
    if let Some(shared_error) = &options.shared_error {
        quote! { #shared_error }
    } else {
        quote! { #error_enum }
    }
}

fn endpoint_error_type_tokens(options: &ModuleOptions, error_name: &Ident) -> TokenStream2 {
    if let Some(shared_error) = &options.shared_error {
        quote! { #shared_error }
    } else {
        quote! { #error_name }
    }
}

fn endpoint_protocol_error_tokens(
    options: &ModuleOptions,
    error_name: &Ident,
    message: TokenStream2,
) -> TokenStream2 {
    if let Some(protocol_error) = &options.protocol_error {
        quote! { #protocol_error(#message) }
    } else {
        quote! { #error_name::Other(#message) }
    }
}

fn module_protocol_error_tokens(
    options: &ModuleOptions,
    error_enum: &Ident,
    message: TokenStream2,
) -> TokenStream2 {
    if let Some(protocol_error) = &options.protocol_error {
        quote! { #protocol_error(#message) }
    } else {
        quote! { #error_enum::Other(#message) }
    }
}

fn expand_impl_trait(
    attrs: &[Attribute],
    trait_name: &Ident,
    method_name: &Ident,
    request_name: &Ident,
    input_name: Option<&Ident>,
    output_name: &Ident,
    reply_name: &Ident,
    error_ty: &TokenStream2,
    streaming_output: bool,
) -> TokenStream2 {
    let input_arg = input_name.map(|input_name| {
        quote! { input: ::tokio::sync::mpsc::Receiver<#input_name>, }
    });
    let output_arg = if streaming_output {
        Some(quote! { output: ::tokio::sync::mpsc::Sender<#output_name>, })
    } else {
        None
    };

    quote! {
        #( #attrs )*
        #[::async_trait::async_trait]
        pub trait #trait_name: Send + Sync {
            /// Handles the generated request payload for this endpoint.
            async fn #method_name(
                &self,
                request: #request_name,
                #input_arg
                #output_arg
            ) -> Result<#reply_name, #error_ty>;
        }
    }
}

fn expand_transport_impl(
    module_options: &ModuleOptions,
    transport_trait: &Ident,
    impl_name: &Ident,
    method_name: &Ident,
    request_name: &Ident,
    input_name: Option<&Ident>,
    output_name: &Ident,
    reply_name: &Ident,
    error_name: &Ident,
    request_enum: &Ident,
    input_enum: &Ident,
    output_enum: &Ident,
    reply_enum: &Ident,
    variant_name: &Ident,
    streaming: bool,
) -> TokenStream2 {
    let endpoint_error_ty = endpoint_error_type_tokens(module_options, error_name);
    let reply_mismatch_error = endpoint_protocol_error_tokens(
        module_options,
        error_name,
        quote! {
            format!(
                "expected {} reply but received {}",
                stringify!(#variant_name),
                other.kind()
            )
        },
    );

    if streaming {
        let input_name = input_name.expect("streaming transport impl requires an input type");
        let input_forward_error = endpoint_protocol_error_tokens(
            module_options,
            error_name,
            quote! {
                format!(
                    "streaming input receiver for {} was dropped before all items were forwarded",
                    stringify!(#variant_name)
                )
            },
        );
        let output_forward_error = endpoint_protocol_error_tokens(
            module_options,
            error_name,
            quote! {
                format!(
                    "streaming output receiver for {} was dropped before all items were forwarded",
                    stringify!(#variant_name)
                )
            },
        );
        let output_mismatch_error = endpoint_protocol_error_tokens(
            module_options,
            error_name,
            quote! {
                format!(
                    "expected {} output but received {}",
                    stringify!(#variant_name),
                    other.kind()
                )
            },
        );

        quote! {
            #[::async_trait::async_trait]
            impl<T> #impl_name for T
            where
                T: #transport_trait + Send + Sync,
            {
                async fn #method_name(
                    &self,
                    request: #request_name,
                    mut input: ::tokio::sync::mpsc::Receiver<#input_name>,
                    output: ::tokio::sync::mpsc::Sender<#output_name>,
                ) -> Result<#reply_name, #endpoint_error_ty> {
                    let (transport_input_tx, transport_input_rx) = ::tokio::sync::mpsc::channel(16);
                    let (transport_output_tx, mut transport_output_rx) =
                        ::tokio::sync::mpsc::channel(16);

                    let forward_input = async move {
                        while let Some(value) = input.recv().await {
                            transport_input_tx
                                .send(#input_enum::#variant_name(value))
                                .await
                                .map_err(|_| #input_forward_error)?;
                        }
                        Ok::<(), #endpoint_error_ty>(())
                    };

                    let forward_output = async move {
                        while let Some(value) = transport_output_rx.recv().await {
                            match value {
                                #output_enum::#variant_name(output_value) => {
                                    output
                                        .send(output_value)
                                        .await
                                        .map_err(|_| #output_forward_error)?;
                                }
                                other => return Err(#output_mismatch_error),
                            }
                        }
                        Ok::<(), #endpoint_error_ty>(())
                    };

                    let reply = self.transport_call(
                        #request_enum::#variant_name(request),
                        Some(transport_input_rx),
                        Some(transport_output_tx),
                    );

                    let (input_result, reply, output_result) =
                        ::tokio::join!(forward_input, reply, forward_output);
                    input_result?;
                    let reply = reply?;
                    output_result?;

                    match reply {
                        #reply_enum::#variant_name(reply) => Ok(reply),
                        other => Err(#reply_mismatch_error),
                    }
                }
            }
        }
    } else {
        quote! {
            #[::async_trait::async_trait]
            impl<T> #impl_name for T
            where
                T: #transport_trait + Send + Sync,
            {
                async fn #method_name(&self, request: #request_name) -> Result<#reply_name, #endpoint_error_ty> {
                    let reply = self
                        .transport_call(#request_enum::#variant_name(request), None, None)
                        .await?;
                    match reply {
                        #reply_enum::#variant_name(reply) => Ok(reply),
                        other => Err(#reply_mismatch_error),
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_transport_dispatch_arm(
    module_options: &ModuleOptions,
    variant_name: &Ident,
    method_name: &Ident,
    request_enum: &Ident,
    input_enum: &Ident,
    output_enum: &Ident,
    reply_enum: &Ident,
    error_enum: &Ident,
    _request_name: &Ident,
    input_name: Option<&Ident>,
    output_name: &Ident,
    _reply_name: &Ident,
    streaming: bool,
) -> TokenStream2 {
    let module_error_ty = module_error_type_tokens(module_options, error_enum);
    if streaming {
        let input_name = input_name.expect("streaming dispatch arm requires an input type");
        let missing_output_error = module_protocol_error_tokens(
            module_options,
            error_enum,
            quote! {
                format!(
                    "missing transport output channel for {}",
                    stringify!(#variant_name)
                )
            },
        );
        let input_forward_error = module_protocol_error_tokens(
            module_options,
            error_enum,
            quote! {
                format!(
                    "transport input for {} was dropped before all items were forwarded",
                    stringify!(#variant_name)
                )
            },
        );
        let input_mismatch_error = module_protocol_error_tokens(
            module_options,
            error_enum,
            quote! {
                format!(
                    "expected {} input but received {}",
                    stringify!(#variant_name),
                    other.kind()
                )
            },
        );
        let output_forward_error = module_protocol_error_tokens(
            module_options,
            error_enum,
            quote! {
                format!(
                    "transport output for {} was dropped before all items were forwarded",
                    stringify!(#variant_name)
                )
            },
        );
        let dispatch_map_err = if module_options.shared_error.is_some() {
            TokenStream2::new()
        } else {
            quote! { .map_err(#error_enum::from) }
        };

        quote! {
            #request_enum::#variant_name(request) => {
                let output = output.ok_or_else(|| #missing_output_error)?;
                let (typed_input_tx, typed_input_rx) = ::tokio::sync::mpsc::channel::<#input_name>(16);
                let (typed_output_tx, mut typed_output_rx) =
                    ::tokio::sync::mpsc::channel::<#output_name>(16);

                let forward_input = async move {
                    if let Some(mut input) = input {
                        while let Some(value) = input.recv().await {
                            match value {
                                #input_enum::#variant_name(input_value) => {
                                    typed_input_tx
                                        .send(input_value)
                                        .await
                                        .map_err(|_| #input_forward_error)?;
                                }
                                other => return Err(#input_mismatch_error),
                            }
                        }
                    }
                    Ok::<(), #module_error_ty>(())
                };

                let forward_output = async move {
                    while let Some(value) = typed_output_rx.recv().await {
                        output
                            .send(#output_enum::#variant_name(value))
                            .await
                            .map_err(|_| #output_forward_error)?;
                    }
                    Ok::<(), #module_error_ty>(())
                };

                let dispatch = async {
                    ctl.#method_name(request, typed_input_rx, typed_output_tx)
                        .await
                        .map(#reply_enum::#variant_name)
                        #dispatch_map_err
                };

                let (input_result, reply, output_result) =
                    ::tokio::join!(forward_input, dispatch, forward_output);
                input_result?;
                let reply = reply?;
                output_result?;
                Ok(reply)
            }
        }
    } else {
        let dispatch_map_err = if module_options.shared_error.is_some() {
            TokenStream2::new()
        } else {
            quote! { .map_err(#error_enum::from) }
        };

        quote! {
            #request_enum::#variant_name(request) => {
                ctl.#method_name(request)
                    .await
                    .map(#reply_enum::#variant_name)
                    #dispatch_map_err
            }
        }
    }
}

fn expand_cli_field(field: &FieldSpec) -> TokenStream2 {
    let attrs = &field.attrs;
    let ident = &field.ident;
    let ty = if field.optional {
        let inner = &field.ty;
        quote! { ::std::option::Option<#inner> }
    } else {
        let ty = &field.ty;
        quote! { #ty }
    };
    let default_arg_attr = if has_clap_arg_attr(attrs) {
        None
    } else {
        Some(quote! { #[arg(long)] })
    };

    quote! {
        #( #attrs )*
        #default_arg_attr
        pub #ident: #ty,
    }
}

fn expand_unary_run_arm(
    command_name: &Ident,
    variant_name: &Ident,
    request_name: &Ident,
    call_enum: &Ident,
    request_block: &FieldBlock,
) -> TokenStream2 {
    let fields = request_block.fields.iter().map(|field| {
        let ident = &field.ident;
        quote! { #ident: args.#ident, }
    });

    quote! {
        #command_name::#variant_name(args) => {
            let reply = ctl
                .dispatch(#call_enum::#variant_name(#request_name {
                    #( #fields )*
                }))
                .await?;
            Self::print_reply(json, &reply).await?;
        }
    }
}

fn expand_streaming_run_arm(
    command_name: &Ident,
    variant_name: &Ident,
    request_name: &Ident,
    call_enum: &Ident,
    input_name: Option<&Ident>,
    output_name: &Ident,
    output_enum: &Ident,
    endpoint: &Endpoint,
) -> TokenStream2 {
    let request_fields = endpoint.request.fields.iter().map(|field| {
        let ident = &field.ident;
        quote! { #ident: args.#ident, }
    });

    let enqueue_input = if let (Some(input_name), Some(input_block)) = (input_name, &endpoint.input) {
        if input_block.fields.len() == 1 && is_vec_u8(&input_block.fields[0].ty) {
            let field_name = &input_block.fields[0].ident;
            quote! {
                let mut stdin_bytes = Vec::new();
                ::tokio::io::stdin().read_to_end(&mut stdin_bytes).await?;
                if !stdin_bytes.is_empty() {
                    input_tx
                        .send(#input_name { #field_name: stdin_bytes })
                        .await
                        .map_err(|_| {
                            ::std::io::Error::new(
                                ::std::io::ErrorKind::BrokenPipe,
                                "input receiver dropped before stdin could be forwarded",
                            )
                        })?;
                }
            }
        } else {
            quote! {}
        }
    } else {
        quote! {}
    };

    let input_receiver = if input_name.is_some() {
        quote! { input: input_rx, }
    } else {
        TokenStream2::new()
    };

    quote! {
        #command_name::#variant_name(args) => {
            use ::tokio::io::AsyncReadExt;

            let (input_tx, input_rx) = ::tokio::sync::mpsc::channel(16);
            let (output_tx, mut output_rx) = ::tokio::sync::mpsc::channel::<#output_name>(16);

            #enqueue_input
            drop(input_tx);

            let dispatch = ctl.dispatch(#call_enum::#variant_name {
                request: #request_name {
                    #( #request_fields )*
                },
                #input_receiver
                output: output_tx,
            });

            let output_task = async {
                while let Some(output_value) = output_rx.recv().await {
                    let output = #output_enum::#variant_name(output_value);
                    Self::print_output(json, &output).await?;
                }
                Ok::<(), Box<dyn ::std::error::Error + Send + Sync>>(())
            };

            let (reply, output_result) = ::tokio::join!(dispatch, output_task);
            output_result?;
            let reply = reply?;
            Self::print_reply(json, &reply).await?;
        }
    }
}

fn expand_print_output_arm(
    output_enum: &Ident,
    variant_name: &Ident,
    endpoint: &Endpoint,
) -> TokenStream2 {
    let raw_stream = endpoint.output.fields.len() == 2
        && endpoint.output.fields.iter().any(|field| field.ident == "is_stdout")
        && endpoint.output.fields.iter().any(|field| field.ident == "output")
        && endpoint
            .output
            .fields
            .iter()
            .find(|field| field.ident == "output")
            .map(|field| is_vec_u8(&field.ty))
            .unwrap_or(false);

    if raw_stream {
        quote! {
            #output_enum::#variant_name(value) => {
                if json {
                    Self::print_json(output)?;
                } else if value.is_stdout {
                    use ::tokio::io::AsyncWriteExt;
                    let mut stdout = ::tokio::io::stdout();
                    stdout.write_all(&value.output).await?;
                    stdout.flush().await?;
                } else {
                    use ::tokio::io::AsyncWriteExt;
                    let mut stderr = ::tokio::io::stderr();
                    stderr.write_all(&value.output).await?;
                    stderr.flush().await?;
                }
            }
        }
    } else {
        quote! {
            #output_enum::#variant_name(value) => {
                if json {
                    Self::print_json(output)?;
                } else {
                    println!("{value:#?}");
                }
            }
        }
    }
}

fn expand_print_reply_arm(reply_enum: &Ident, variant_name: &Ident, endpoint: &Endpoint) -> TokenStream2 {
    let reply_block = endpoint.reply.as_ref().unwrap_or(&endpoint.output);
    let exit_code_only =
        reply_block.fields.len() == 1 && reply_block.fields.iter().any(|field| field.ident == "exit_code");
    let exit_code_with_streams = reply_block.fields.len() == 3
        && reply_block.fields.iter().any(|field| field.ident == "exit_code")
        && reply_block.fields.iter().any(|field| field.ident == "stdout")
        && reply_block.fields.iter().any(|field| field.ident == "stderr")
        && reply_block
            .fields
            .iter()
            .find(|field| field.ident == "stdout")
            .map(|field| is_vec_u8(&field.ty))
            .unwrap_or(false)
        && reply_block
            .fields
            .iter()
            .find(|field| field.ident == "stderr")
            .map(|field| is_vec_u8(&field.ty))
            .unwrap_or(false);
    let ok_reply = reply_block.fields.iter().any(|field| field.ident == "ok")
        && reply_block.fields.iter().any(|field| field.ident == "error");
    let ok_reply_only = ok_reply && reply_block.fields.len() == 2;

    if exit_code_only {
        quote! {
            #reply_enum::#variant_name(value) => {
                if json {
                    Self::print_json(reply)?;
                } else if value.exit_code != 0 {
                    return Err(Box::new(::std::io::Error::new(
                        ::std::io::ErrorKind::Other,
                        format!("exit code: {}", value.exit_code),
                    )));
                }
            }
        }
    } else if exit_code_with_streams {
        quote! {
            #reply_enum::#variant_name(value) => {
                if json {
                    Self::print_json(reply)?;
                } else {
                    use ::tokio::io::AsyncWriteExt;

                    if !value.stdout.is_empty() {
                        let mut stdout = ::tokio::io::stdout();
                        stdout.write_all(&value.stdout).await?;
                        stdout.flush().await?;
                    }

                    if !value.stderr.is_empty() {
                        let mut stderr = ::tokio::io::stderr();
                        stderr.write_all(&value.stderr).await?;
                        stderr.flush().await?;
                    }

                    if value.exit_code != 0 {
                        return Err(Box::new(::std::io::Error::new(
                            ::std::io::ErrorKind::Other,
                            format!("exit code: {}", value.exit_code),
                        )));
                    }
                }
            }
        }
    } else if ok_reply {
        let success_body = if ok_reply_only {
            quote! { println!("ok"); }
        } else {
            quote! { println!("{value:#?}"); }
        };

        quote! {
            #reply_enum::#variant_name(value) => {
                if json {
                    Self::print_json(reply)?;
                } else if !value.ok {
                    return Err(Box::new(::std::io::Error::new(
                        ::std::io::ErrorKind::Other,
                        value
                            .error
                            .clone()
                            .unwrap_or_else(|| "request failed".to_string()),
                    )));
                } else {
                    #success_body
                }
            }
        }
    } else {
        quote! {
            #reply_enum::#variant_name(value) => {
                if json {
                    Self::print_json(reply)?;
                } else {
                    println!("{value:#?}");
                }
            }
        }
    }
}

fn pascal_case_ident(ident: &Ident) -> Ident {
    let mut rendered = String::new();
    for segment in ident.to_string().split('_') {
        if segment.is_empty() {
            continue;
        }
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            rendered.extend(first.to_uppercase());
            rendered.push_str(chars.as_str());
        }
    }
    Ident::new(&rendered, ident.span())
}

fn humanize_ident(ident: &Ident) -> String {
    let value = ident.to_string();
    let mut rendered = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index > 0 && ch.is_uppercase() {
            rendered.push(' ');
        }
        rendered.push(ch.to_ascii_lowercase());
    }
    rendered
}

fn last_path_ident(path: &Path) -> Ident {
    path.segments
        .last()
        .map(|segment| segment.ident.clone())
        .expect("path has at least one segment")
}

fn has_clap_arg_attr(attrs: &[Attribute]) -> bool {
    attrs.iter()
        .any(|attr| attr.path().is_ident("arg") || attr.path().is_ident("command"))
}

fn filter_clap_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs.iter()
        .filter(|attr| !attr.path().is_ident("arg") && !attr.path().is_ident("command"))
        .cloned()
        .collect()
}

fn is_vec_u8(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let Some(segment) = type_path.path.segments.last() else {
        return false;
    };
    if segment.ident != "Vec" {
        return false;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    let Some(syn::GenericArgument::Type(Type::Path(inner))) = arguments.args.first() else {
        return false;
    };
    inner.path.is_ident("u8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::parse2;

    #[test]
    fn parses_optional_and_streaming_endpoints() {
        let module: CtlModule = parse2(quote! {
            mod mirage_ctl {
                time({
                    session_id: String = None,
                }) -> {
                    session_time: Time,
                } ? SessionNotFound;

                attach({
                    exec_id: String,
                }) ... {
                    stream: Vec<u8>,
                } -> {
                    is_stdout: bool,
                    output: Vec<u8>,
                } => {
                    exit_code: i32,
                };
            }
        })
        .expect("module should parse");

        assert_eq!(module.endpoints.len(), 2);
        assert!(module.endpoints[0].request.fields[0].optional);
        assert!(module.endpoints[1].input.is_some());
        assert!(module.endpoints[1].reply.is_some());
    }

    #[test]
    fn parses_module_and_endpoint_options() {
        let module: CtlModule = parse2(quote! {
            #[ctl(
                error = crate::ctl::MirageDaemonError,
                protocol_error = crate::ctl::MirageDaemonError::protocol,
                cli_name = "mirage-ctl"
            )]
            pub mod daemon {
                #[ctl(skip_cli)]
                register({
                    info: String,
                }) -> {
                    ok: bool,
                };
            }
        })
        .expect("module should parse");

        let (_, module_options) =
            split_module_attrs(module.attrs.clone()).expect("module attrs should parse");
        assert_eq!(
            module_options.shared_error.as_ref().unwrap().segments.last().unwrap().ident,
            "MirageDaemonError"
        );
        assert_eq!(
            module_options
                .protocol_error
                .as_ref()
                .unwrap()
                .segments
                .last()
                .unwrap()
                .ident,
            "protocol"
        );
        assert_eq!(module_options.cli_name.as_deref(), Some("mirage-ctl"));

        let endpoint_attrs = module.endpoints[0].attrs.clone();
        let (_, endpoint_options) =
            split_endpoint_attrs(&endpoint_attrs).expect("endpoint attrs should parse");
        assert!(endpoint_options.skip_cli);
    }

    #[test]
    fn strips_clap_attrs_from_protocol_struct_fields() {
        let block: FieldBlock = parse2(quote! {
            {
                /// Command to run.
                #[arg(last = true, required = true)]
                command: Vec<String>,
            }
        })
        .expect("field block should parse");

        let struct_tokens = expand_struct(&[], &format_ident!("ExecRequest"), &block).to_string();
        let cli_tokens = expand_cli_field(&block.fields[0]).to_string();

        assert!(!struct_tokens.contains("# [ arg"));
        assert!(cli_tokens.contains("last = true"));
        assert!(cli_tokens.contains("required = true"));
    }

    #[test]
    fn expands_transport_and_skips_hidden_cli_commands() {
        let tokens = expand_ctl_module(
            parse2(quote! {
                #[ctl(
                    error = crate::ctl::MirageDaemonError,
                    protocol_error = crate::ctl::MirageDaemonError::protocol,
                    cli_name = "mirage-ctl"
                )]
                pub mod daemon {
                    #[ctl(skip_cli)]
                    register({
                        info: String,
                    }) -> {
                        ok: bool,
                    };

                    status({}) -> {
                        ok: bool,
                    };
                }
            })
            .expect("module should parse"),
        )
        .expect("module should expand")
        .to_string();

        assert!(tokens.contains("pub trait DaemonTransport"));
        assert!(tokens.contains("pub async fn dispatch_transport"));
        assert!(tokens.contains("mirage-ctl"));
        assert!(tokens.contains("StatusCommandArgs"));
        assert!(!tokens.contains("RegisterCommandArgs"));
    }
}
