pub use strum::{Display, FromRepr};

#[macro_export]
macro_rules! impl_error_and_result {
    {
        $error:ident,
        $result:ident,
        [$($variant:ident),*]
    } => {
        #[derive(Debug, PartialEq, Eq, $crate::Display)]
        pub enum $error {
            $($variant),*
        }

        $crate::impl_result! {
            $result,
            [Ok $(, $variant)*]
        }

        impl From<Result<(), $error>> for $result {
            fn from(value: Result<(), $error>) -> Self {
                match value {
                    Ok(()) => Self::Ok,
                    $(Err($error::$variant) => Self::$variant),*
                }
            }
        }

        impl From<$result> for Result<(), $error> {
            fn from(value: $result) -> Self {
                match value {
                    $result::Ok => Ok(()),
                    $($result::$variant => Err($error::$variant)),*
                }
            }
        }

        impl std::error::Error for $error {
        }
    };
}

#[macro_export]
macro_rules! impl_result {
    {
        $result:ident,
        [$($variant:ident),+]
    } => {
        #[derive(Debug, $crate::FromRepr, PartialEq, Eq)]
        #[repr(u8)]
        #[must_use]
        pub enum $result {
            $($variant),+
        }

        impl From<$result> for u8 {
            fn from(value: $result) -> Self {
                value as u8
            }
        }
    };
}
