use crate::common::Time;

macro_rules! ctl_dsl {
    (
        $ctl:ident {
            $($body:tt)*
        }
    ) => {
        ctl_dsl!(@emit_items $($body)*);

        ::paste::paste! {
            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub enum [< $ctl Request >] {
                ctl_dsl!(@emit_request_variants $($body)*)
            }

            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub enum [< $ctl Input >] {
                ctl_dsl!(@emit_input_variants $($body)*)
            }

            impl [< $ctl Input >] {
                pub fn kind(&self) -> &'static str {
                    match self {
                        ctl_dsl!(@emit_input_kind_arms $($body)*)
                    }
                }
            }

            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub enum [< $ctl Output >] {
                ctl_dsl!(@emit_output_variants $($body)*)
            }

            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub enum [< $ctl Reply >] {
                ctl_dsl!(@emit_reply_variants $($body)*)
            }

            #[derive(
                Debug,
                Clone,
                PartialEq,
                Eq,
                ::serde::Serialize,
                ::serde::Deserialize,
                ::thiserror::Error,
            )]
            pub enum [< $ctl DispatchError >] {
                #[error("unexpected input variant {found} for method {expected}")]
                UnexpectedInputVariant {
                    expected: String,
                    found: String,
                },
                #[error("unexpected streamed input for non-streaming method {method}")]
                UnexpectedInputs { method: String },
            }

            #[derive(
                Debug,
                Clone,
                PartialEq,
                Eq,
                ::serde::Serialize,
                ::serde::Deserialize,
                ::thiserror::Error,
            )]
            pub enum [< $ctl Error >] {
                #[error(transparent)]
                Dispatch(#[from] [< $ctl DispatchError >]),
                ctl_dsl!(@emit_error_variants $($body)*)
            }

            pub trait [< $ctl Impl >]: ctl_dsl!(@emit_impl_trait_bounds $($body)*) Send + Sync {
                fn dispatch(
                    &self,
                    request: [< $ctl Request >],
                    inputs: Vec<[< $ctl Input >]>,
                ) -> Result<(Vec<[< $ctl Output >]>, [< $ctl Reply >]), [< $ctl Error >]> {
                    match request {
                        ctl_dsl!(@emit_dispatch_arms $ctl $($body)*)
                    }
                }
            }

            impl<T> [< $ctl Impl >] for T
            where
                T: ctl_dsl!(@emit_impl_trait_bounds $($body)*) Send + Sync,
            {
            }
        }
    };

    (@emit_items) => {};

    (@emit_items
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        ::paste::paste! {
            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub struct [< $name:camel Request >] {
                ctl_dsl!(@emit_struct_fields $($request_fields)*)
            }

            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub struct [< $name:camel Input >] {
                ctl_dsl!(@emit_struct_fields $($input_fields)*)
            }

            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub struct [< $name:camel Output >] {
                ctl_dsl!(@emit_struct_fields $($output_fields)*)
            }

            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub struct [< $name:camel Reply >] {
                ctl_dsl!(@emit_struct_fields $($reply_fields)*)
            }

            ctl_dsl!(@emit_error_items $name $(? $($errors)|+)?);

            pub trait [< Impl $name:camel >]: Send + Sync {
                fn $name(
                    &self,
                    request: [< $name:camel Request >],
                    inputs: Vec<[< $name:camel Input >]>,
                ) -> Result<
                    (Vec<[< $name:camel Output >]>, [< $name:camel Reply >]),
                    [< $name:camel Error >],
                >;
            }
        }

        ctl_dsl!(@emit_items $($rest)*);
    };

    (@emit_items
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        ::paste::paste! {
            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub struct [< $name:camel Request >] {
                ctl_dsl!(@emit_struct_fields $($request_fields)*)
            }

            #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub struct [< $name:camel Reply >] {
                ctl_dsl!(@emit_struct_fields $($reply_fields)*)
            }

            ctl_dsl!(@emit_error_items $name $(? $($errors)|+)?);

            pub trait [< Impl $name:camel >]: Send + Sync {
                fn $name(
                    &self,
                    request: [< $name:camel Request >],
                ) -> Result<[< $name:camel Reply >], [< $name:camel Error >]>;
            }
        }

        ctl_dsl!(@emit_items $($rest)*);
    };

    (@emit_struct_fields) => {};

    (@emit_struct_fields
        $(#[$field_meta:meta])*
        $field:ident : $ty:ty = None,
        $($rest:tt)*
    ) => {
        $(#[$field_meta])*
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub $field: Option<$ty>,
        ctl_dsl!(@emit_struct_fields $($rest)*)
    };

    (@emit_struct_fields
        $(#[$field_meta:meta])*
        $field:ident : $ty:ty,
        $($rest:tt)*
    ) => {
        $(#[$field_meta])*
        pub $field: $ty,
        ctl_dsl!(@emit_struct_fields $($rest)*)
    };

    (@emit_error_items $name:ident ? $($error:ident)|+) => {
        $(
            #[derive(
                Debug,
                Clone,
                PartialEq,
                Eq,
                ::serde::Serialize,
                ::serde::Deserialize,
                ::thiserror::Error,
            )]
            #[error("{}", stringify!($error))]
            pub struct $error;
        )*

        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            ::serde::Serialize,
            ::serde::Deserialize,
            ::thiserror::Error,
        )]
        pub enum [< $name:camel Error >] {
            $(
                #[error(transparent)]
                $error(#[from] $error),
            )*
        }
    };

    (@emit_error_items $name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
        pub enum [< $name:camel Error >] {}

        impl ::core::fmt::Display for [< $name:camel Error >] {
            fn fmt(&self, _formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match *self {}
            }
        }

        impl ::std::error::Error for [< $name:camel Error >] {}
    };

    (@emit_request_variants) => {};

    (@emit_request_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $name:camel >]([< $name:camel Request >]),
        ctl_dsl!(@emit_request_variants $($rest)*)
    };

    (@emit_request_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $name:camel >]([< $name:camel Request >]),
        ctl_dsl!(@emit_request_variants $($rest)*)
    };

    (@emit_input_variants) => {};

    (@emit_input_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $name:camel >]([< $name:camel Input >]),
        ctl_dsl!(@emit_input_variants $($rest)*)
    };

    (@emit_input_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        ctl_dsl!(@emit_input_variants $($rest)*)
    };

    (@emit_input_kind_arms) => {};

    (@emit_input_kind_arms
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        Self::[< $name:camel >](_) => stringify!($name),
        ctl_dsl!(@emit_input_kind_arms $($rest)*)
    };

    (@emit_input_kind_arms
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        ctl_dsl!(@emit_input_kind_arms $($rest)*)
    };

    (@emit_output_variants) => {};

    (@emit_output_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $name:camel >]([< $name:camel Output >]),
        ctl_dsl!(@emit_output_variants $($rest)*)
    };

    (@emit_output_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        ctl_dsl!(@emit_output_variants $($rest)*)
    };

    (@emit_reply_variants) => {};

    (@emit_reply_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $name:camel >]([< $name:camel Reply >]),
        ctl_dsl!(@emit_reply_variants $($rest)*)
    };

    (@emit_reply_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $name:camel >]([< $name:camel Reply >]),
        ctl_dsl!(@emit_reply_variants $($rest)*)
    };

    (@emit_error_variants) => {};

    (@emit_error_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        #[error(transparent)]
        [< $name:camel >](#[from] [< $name:camel Error >]),
        ctl_dsl!(@emit_error_variants $($rest)*)
    };

    (@emit_error_variants
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        #[error(transparent)]
        [< $name:camel >](#[from] [< $name:camel Error >]),
        ctl_dsl!(@emit_error_variants $($rest)*)
    };

    (@emit_impl_trait_bounds) => {};

    (@emit_impl_trait_bounds
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< Impl $name:camel >] + ctl_dsl!(@emit_impl_trait_bounds $($rest)*)
    };

    (@emit_impl_trait_bounds
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< Impl $name:camel >] + ctl_dsl!(@emit_impl_trait_bounds $($rest)*)
    };

    (@emit_dispatch_arms $ctl:ident) => {};

    (@emit_dispatch_arms $ctl:ident
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) ... {
            $($input_fields:tt)*
        } -> {
            $($output_fields:tt)*
        } => {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $ctl Request >]::[< $name:camel >](request) => {
            let typed_inputs = inputs
                .into_iter()
                .map(|input| match input {
                    [< $ctl Input >]::[< $name:camel >](input) => Ok(input),
                    other => Err([< $ctl Error >]::Dispatch([< $ctl DispatchError >]::UnexpectedInputVariant {
                        expected: stringify!($name).to_string(),
                        found: other.kind().to_string(),
                    })),
                })
                .collect::<Result<Vec<_>, _>>()?;

            self.$name(request, typed_inputs)
                .map(|(outputs, reply)| {
                    (
                        outputs.into_iter().map([< $ctl Output >]::[< $name:camel >]).collect(),
                        [< $ctl Reply >]::[< $name:camel >](reply),
                    )
                })
                .map_err([< $ctl Error >]::[< $name:camel >])
        },
        ctl_dsl!(@emit_dispatch_arms $ctl $($rest)*)
    };

    (@emit_dispatch_arms $ctl:ident
        $(#[$meta:meta])*
        $name:ident({
            $($request_fields:tt)*
        }) -> {
            $($reply_fields:tt)*
        } $(? $($errors:ident)|+)?;
        $($rest:tt)*
    ) => {
        [< $ctl Request >]::[< $name:camel >](request) => {
            if !inputs.is_empty() {
                return Err([< $ctl Error >]::Dispatch([< $ctl DispatchError >]::UnexpectedInputs {
                    method: stringify!($name).to_string(),
                }));
            }

            self.$name(request)
                .map(|reply| (Vec::new(), [< $ctl Reply >]::[< $name:camel >](reply)))
                .map_err([< $ctl Error >]::[< $name:camel >])
        },
        ctl_dsl!(@emit_dispatch_arms $ctl $($rest)*)
    };
}

ctl_dsl!{
    MirageCtl {
        /// Attach to a running process and optionally send data to its `stdin`.
        ///
        /// The server will read from [`stream`](Self::stream) and write to the
        /// exec's `stdin`. It responds with a stream of [`AttachReply`] messages
        /// carrying `stdout` / `stderr` chunks and eventually a [`RunExit`].
        attach({
            /// Identifier of the exec to attach to.
            exec_id: String,
        }) ... {
            /// Data to write to the exec's `stdin`.
            stream: Vec<u8>,
        } -> {
            /// Whether this chunk came from `stdout` or `stderr`.
            is_stdout: bool,
            /// Chunks of data read from the exec's `stdout` or `stderr`.
            output: Vec<u8>,
        } => {
            /// The exit code of the exec after it finishes.
            exit_code: i32,
        };

        /// Request the current simulated time for a session.
        time({
            /// session identifier to query the time for.
            session_id: String = None,
        }) -> {
            /// The current simulated time for the session, in nanoseconds. 
            session_time : Time
        } ? SessionNotFound;
    }
}
