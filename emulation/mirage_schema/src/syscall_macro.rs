//! The [`syscall_dsl!`] declarative DSL for the additional, non-ioctl
//! POSIX calls the mirage shim intercepts.
//!
//! Conceptually identical to [`ioctl_dsl!`](crate::ioctl_dsl): for each
//! listed syscall name it emits
//!
//! * `Handle$SubsysSyscalls` — a trait with one method per syscall.
//! * `$Name_Request` / `$Name_Response` structs.
//! * `Any$SubsysSyscallRequest` / `Any$SubsysSyscallResponse` dispatch
//!   enums.
//! * `HandleAny$SubsysSyscalls` — dynamic-dispatch trait.
//! * `Forward$SubsysSyscalls` — blanket-implemented forwarding trait so
//!   a remote proxy is one method instead of many.

/// Declarative DSL for grouped syscall interfaces.
#[macro_export]
macro_rules! syscall_dsl {
    (
        $(
            $subsys:ident {
                $(
                    $(#[$meta:meta])*
                    $name:ident {
                        $(
                            $(#[$args_field_meta:meta])*
                            $args_field:ident : $args_ty:ty
                        ),* $(,)?
                    } => {
                        $(
                            $(#[$rets_field_meta:meta])*
                            $rets_field:ident : $rets_ty:ty
                        ),* $(,)?
                    };
                )*
            }
        )*
    ) => {
        $(
            ::paste::paste! {
                $(
                    $(#[$meta])*
                    #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                    pub struct [< $name Request >] {
                        $(
                            $(#[$args_field_meta])*
                            pub $args_field: $args_ty,
                        )*
                    }

                    $(#[$meta])*
                    #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                    pub struct [< $name Response >] {
                        $(
                            $(#[$rets_field_meta])*
                            pub $rets_field: $rets_ty,
                        )*
                    }
                )*

                pub trait [< Handle $subsys:camel Syscalls >] : Send + Sync {
                    $(
                        fn [< $name:snake >](
                            &self,
                            ctx: $crate::amdgpu::IoctlCtx,
                            request: [< $name Request >],
                        ) -> $crate::amdgpu_error::AmdgpuResult<[< $name Response >]>;
                    )*
                }

                #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                pub enum [< Any $subsys:camel SyscallRequest >] {
                    $(
                        $name([< $name Request >]),
                    )*
                }

                #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                pub enum [< Any $subsys:camel SyscallResponse >] {
                    $(
                        $name([< $name Response >]),
                    )*
                }

                pub trait [< HandleAny $subsys:camel Syscalls >] : Send + Sync {
                    fn [< handle_any_ $subsys _syscall >](
                        &self,
                        ctx: $crate::amdgpu::IoctlCtx,
                        request: [< Any $subsys:camel SyscallRequest >],
                    ) -> $crate::amdgpu_error::AmdgpuResult<[< Any $subsys:camel SyscallResponse >]>;
                }

                impl<T: [< Handle $subsys:camel Syscalls >]> [< HandleAny $subsys:camel Syscalls >] for T {
                    fn [< handle_any_ $subsys _syscall >](
                        &self,
                        ctx: $crate::amdgpu::IoctlCtx,
                        request: [< Any $subsys:camel SyscallRequest >],
                    ) -> $crate::amdgpu_error::AmdgpuResult<[< Any $subsys:camel SyscallResponse >]> {
                        match request {
                            $(
                                [< Any $subsys:camel SyscallRequest >]::$name(req) => {
                                    self.[< $name:snake >](ctx, req).map([< Any $subsys:camel SyscallResponse >]::$name)
                                }
                            )*
                        }
                    }
                }

                pub trait [< Forward $subsys:camel Syscalls >] : Send + Sync {
                    fn [< forward_ $subsys _syscall >](
                        &self,
                        ctx: $crate::amdgpu::IoctlCtx,
                        request: [< Any $subsys:camel SyscallRequest >],
                    ) -> $crate::amdgpu_error::AmdgpuResult<[< Any $subsys:camel SyscallResponse >]>;
                }

                impl<T: [< Forward $subsys:camel Syscalls >]> [< Handle $subsys:camel Syscalls >] for T {
                    $(
                        fn [< $name:snake >](
                            &self,
                            ctx: $crate::amdgpu::IoctlCtx,
                            request: [< $name Request >],
                        ) -> $crate::amdgpu_error::AmdgpuResult<[< $name Response >]> {
                            let req = [< Any $subsys:camel SyscallRequest >]::$name(request);
                            match self.[< forward_ $subsys _syscall >](ctx, req)? {
                                [< Any $subsys:camel SyscallResponse >]::$name(r) => Ok(r),
                                _ => Err($crate::amdgpu_error::AmdgpuError::Invalid),
                            }
                        }
                    )*
                }
            }
        )*
    };
}
