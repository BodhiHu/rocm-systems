//! The [`ioctl!`] declarative DSL for grouped ioctl interfaces.

/// Declarative DSL for grouped ioctl interfaces.
///
/// ```ignore
/// ioctl! {
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
/// Expands (per subsystem) to a module containing the constants, the
/// `*Request` / `*Response` payload structs, a `Handle{Subsys}Ioctl` trait
/// with one method per ioctl, and a `SingleThreadedIoctlHandler<T>` wrapper.
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

                pub trait [< Handle $subsys:camel Ioctl >] {
                    $(
                        fn [< $name:lower >](
                            self,
                            request: [< $name:camel Request >],
                        ) -> $crate::amdgpu_error::AmdgpuResult<[< $name:camel Response >]>;
                    )*
                }
            }
        )*
    };
}

