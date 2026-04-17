use std::collections::BTreeMap;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{
    Attribute, Error, Ident, Path, Result, Token, Type, Visibility, braced, parenthesized,
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

fn expand_ctl_module(module: CtlModule) -> Result<TokenStream2> {
    let CtlModule {
        attrs,
        vis,
        ident,
        endpoints,
    } = module;

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
    let cli_binary_name = ident.to_string().replace('_', "-");

    let mut generated_types = Vec::new();
    let mut error_defs: BTreeMap<String, TokenStream2> = BTreeMap::new();
    let mut trait_defs = Vec::new();
    let mut command_args = Vec::new();
    let mut command_variants = Vec::new();
    let mut request_variants = Vec::new();
    let mut input_variants = Vec::new();
    let mut output_variants = Vec::new();
    let mut reply_variants = Vec::new();
    let mut error_variants = Vec::new();
    let mut call_variants = Vec::new();
    let mut dispatch_arms = Vec::new();
    let mut impl_supertraits = Vec::new();
    let mut run_arms = Vec::new();
    let mut print_output_arms = Vec::new();
    let mut print_reply_arms = Vec::new();

    for endpoint in &endpoints {
        let endpoint_prefix = pascal_case_ident(&endpoint.ident);
        let request_name = format_ident!("{endpoint_prefix}Request");
        let output_name = format_ident!("{endpoint_prefix}Output");
        let reply_name = format_ident!("{endpoint_prefix}Reply");
        let error_name = format_ident!("{endpoint_prefix}Error");
        let impl_name = format_ident!("Impl{endpoint_prefix}");
        let command_args_name = format_ident!("{endpoint_prefix}CommandArgs");
        let variant_name = endpoint_prefix.clone();
        let input_name = endpoint
            .input
            .as_ref()
            .map(|_| format_ident!("{endpoint_prefix}Input"));

        generated_types.push(expand_struct(&endpoint.attrs, &request_name, &endpoint.request));
        generated_types.push(expand_struct(&endpoint.attrs, &output_name, &endpoint.output));

        if let Some(input_block) = &endpoint.input {
            generated_types.push(expand_struct(
                &endpoint.attrs,
                input_name.as_ref().expect("input name must exist"),
                input_block,
            ));
        }

        if let Some(reply_block) = &endpoint.reply {
            generated_types.push(expand_struct(&endpoint.attrs, &reply_name, reply_block));
        } else {
            generated_types.push(quote! {
                pub type #reply_name = #output_name;
            });
        }

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

        trait_defs.push(expand_impl_trait(
            &impl_name,
            &endpoint.ident,
            &request_name,
            input_name.as_ref(),
            &output_name,
            &reply_name,
            &error_name,
            endpoint.reply.is_some(),
        ));
        impl_supertraits.push(quote! { #impl_name });

        request_variants.push(quote! { #variant_name(#request_name), });
        output_variants.push(quote! { #variant_name(#output_name), });
        reply_variants.push(quote! { #variant_name(#reply_name), });
        error_variants.push(quote! {
            #[error(transparent)]
            #variant_name(#[from] #error_name),
        });

        if let Some(input_name) = &input_name {
            input_variants.push(quote! { #variant_name(#input_name), });
        }

        if endpoint.reply.is_some() {
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
                        .map_err(#error_enum::from)
                }
            });

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
            print_output_arms.push(expand_print_output_arm(
                &output_enum,
                &variant_name,
                endpoint,
            ));
        } else {
            call_variants.push(quote! { #variant_name(#request_name), });
            let method = &endpoint.ident;
            dispatch_arms.push(quote! {
                #call_enum::#variant_name(request) => {
                    self.#method(request)
                        .await
                        .map(#reply_enum::#variant_name)
                        .map_err(#error_enum::from)
                }
            });

            run_arms.push(expand_unary_run_arm(
                &command_name,
                &variant_name,
                &request_name,
                &call_enum,
                &endpoint.request,
            ));
        }

        print_reply_arms.push(expand_print_reply_arm(&reply_enum, &variant_name, endpoint));

        let cli_fields = endpoint.request.fields.iter().map(expand_cli_field);
        command_args.push(quote! {
            #[derive(Debug, Clone, ::clap::Args)]
            pub struct #command_args_name {
                #( #cli_fields )*
            }
        });
        let endpoint_attrs = &endpoint.attrs;
        command_variants.push(quote! {
            #( #endpoint_attrs )*
            #variant_name(#command_args_name),
        });
    }

    let error_defs = error_defs.values().collect::<Vec<_>>();

    Ok(quote! {
        #( #attrs )*
        #vis mod #ident {
            #[allow(unused_imports)]
            use super::*;

            #( #generated_types )*
            #( #error_defs )*
            #( #trait_defs )*
            #( #command_args )*

            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #request_enum {
                #( #request_variants )*
            }

            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #input_enum {
                #( #input_variants )*
            }

            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #output_enum {
                #( #output_variants )*
            }

            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #reply_enum {
                #( #reply_variants )*
            }

            #[derive(
                Debug,
                Clone,
                ::serde::Serialize,
                ::serde::Deserialize,
                ::thiserror::Error,
            )]
            pub enum #error_enum {
                #( #error_variants )*
            }

            pub enum #call_enum {
                #( #call_variants )*
            }

            #[::async_trait::async_trait]
            pub trait #impl_trait: #( #impl_supertraits + )* Send + Sync {
                async fn dispatch(&self, call: #call_enum) -> Result<#reply_enum, #error_enum> {
                    match call {
                        #( #dispatch_arms, )*
                    }
                }
            }

            impl<T> #impl_trait for T where T: #( #impl_supertraits + )* Send + Sync {}

            #[derive(Debug, Clone, ::clap::Subcommand)]
            pub enum #command_name {
                #( #command_variants )*
            }

            #[derive(Debug, Clone, ::clap::Parser)]
            #[command(name = #cli_binary_name)]
            pub struct #cli_name {
                #[arg(long, global = true, default_value_os_t = crate::paths::socket_path())]
                pub socket: ::std::path::PathBuf,

                #[arg(long, global = true)]
                pub json: bool,

                #[command(subcommand)]
                pub command: #command_name,
            }

            impl #cli_name {
                pub async fn run_with<T>(self, ctl: &T) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>>
                where
                    T: #impl_trait,
                {
                    let json = self.json;
                    match self.command {
                        #( #run_arms )*
                    }
                    Ok(())
                }

                fn print_json<T>(value: &T) -> Result<(), Box<dyn ::std::error::Error + Send + Sync>>
                where
                    T: ::serde::Serialize,
                {
                    println!("{}", ::serde_json::to_string_pretty(value)?);
                    Ok(())
                }

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
    let fields = block.fields.iter().map(|field| {
        let field_attrs = &field.attrs;
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
        #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
        pub struct #name {
            #( #fields )*
        }
    }
}

fn expand_impl_trait(
    trait_name: &Ident,
    method_name: &Ident,
    request_name: &Ident,
    input_name: Option<&Ident>,
    output_name: &Ident,
    reply_name: &Ident,
    error_name: &Ident,
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
        #[::async_trait::async_trait]
        pub trait #trait_name: Send + Sync {
            async fn #method_name(
                &self,
                request: #request_name,
                #input_arg
                #output_arg
            ) -> Result<#reply_name, #error_name>;
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

    quote! {
        #( #attrs )*
        #[arg(long)]
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
    let exit_code_only = endpoint
        .reply
        .as_ref()
        .map(|reply| reply.fields.len() == 1 && reply.fields.iter().any(|field| field.ident == "exit_code"))
        .unwrap_or(false);

    if exit_code_only {
        quote! {
            #reply_enum::#variant_name(value) => {
                if json {
                    Self::print_json(reply)?;
                } else if value.exit_code != 0 {
                    eprintln!("exit code: {}", value.exit_code);
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
}
