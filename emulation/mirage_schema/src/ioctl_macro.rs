//! The [`ioctl_dsl!`] declarative DSL for grouped ioctl interfaces.

/// Declarative DSL for grouped ioctl interfaces.
///
/// ```ignore
/// ioctl_dsl! {
///     kfd {
///         AMDKFD_IOC_FOO(0x42) {
///             some_field : u32,
///         } => {
///             result : u64,
///         };
///     }
/// }
/// ```
///
/// Expands (per subsystem) to:
///
/// * `pub const $NAME: u32` — the ioctl number.
/// * `pub struct $NameRequest` / `$NameResponse` — request / response payloads.
/// * `Handle{Subsys}Ioctl` — async trait with one method per ioctl (takes `&self` + `IoctlCtx`).
/// * `Any{Subsys}IoctlRequest` / `Any{Subsys}IoctlResponse` — dispatch enums.
/// * `HandleAny{Subsys}Ioctl` — dispatch trait, auto-implemented for `Handle{Subsys}Ioctl`.
#[macro_export]
macro_rules! ioctl_dsl {
    (
        $(
            $subsys:ident {
                $(
                    $(#[$ioctl_meta:meta])*
                    $name:ident ( $nr:expr ) {
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
                    $(#[$ioctl_meta])*
                    pub const $name: u32 = $nr;

                    #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                    pub struct [< $name:camel Request >] {
                        $(
                            $(#[$args_field_meta])*
                            pub $args_field: $args_ty,
                        )*
                    }

                    #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                    pub struct [< $name:camel Response >] {
                        $(
                            $(#[$rets_field_meta])*
                            pub $rets_field: $rets_ty,
                        )*
                    }
                )*

                // --- per-ioctl async trait ---

                #[::async_trait::async_trait]
                pub trait [< Handle $subsys:camel Ioctl >] : Send + Sync {
                    $(
                        async fn [< $name:lower >](
                            &self,
                            ctx: $crate::amdgpu::IoctlCtx,
                            request: [< $name:camel Request >],
                        ) -> $crate::amdgpu_error::AmdgpuResult<[< $name:camel Response >]>;
                    )*
                }

                // --- dispatch enums ---

                #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                #[serde(tag = "ioctl")]
                pub enum [< Any $subsys:camel IoctlRequest >] {
                    $(
                        [< $name:camel >]([< $name:camel Request >]),
                    )*
                }

                #[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
                #[serde(tag = "ioctl")]
                pub enum [< Any $subsys:camel IoctlResponse >] {
                    $(
                        [< $name:camel >]([< $name:camel Response >]),
                    )*
                }

                // --- dispatch trait, blanket-impl for Handle{Subsys}Ioctl ---

                #[::async_trait::async_trait]
                pub trait [< HandleAny $subsys:camel Ioctl >] : Send + Sync {
                    async fn [< handle_any_ $subsys _ioctl >](
                        &self,
                        ctx: $crate::amdgpu::IoctlCtx,
                        request: [< Any $subsys:camel IoctlRequest >],
                    ) -> $crate::amdgpu_error::AmdgpuResult<[< Any $subsys:camel IoctlResponse >]>;
                }

                #[::async_trait::async_trait]
                impl<T: [< Handle $subsys:camel Ioctl >]> [< HandleAny $subsys:camel Ioctl >] for T {
                    async fn [< handle_any_ $subsys _ioctl >](
                        &self,
                        ctx: $crate::amdgpu::IoctlCtx,
                        request: [< Any $subsys:camel IoctlRequest >],
                    ) -> $crate::amdgpu_error::AmdgpuResult<[< Any $subsys:camel IoctlResponse >]> {
                        match request {
                            $(
                                [< Any $subsys:camel IoctlRequest >]::[< $name:camel >](req) => {
                                    self.[< $name:lower >](ctx, req).await.map([< Any $subsys:camel IoctlResponse >]::[< $name:camel >])
                                }
                            )*
                        }
                    }
                }
            }
        )*
    };
}
