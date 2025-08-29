pub use serde;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Add, AddAssign, Sub, SubAssign},
};

pub mod map;
pub use hashbar;

#[doc(hidden)]
pub trait IdType:
    Copy
    + Hash
    + Ord
    + Serialize
    + DeserializeOwned
    + Into<u64>
    + From<u64>
    + AsRef<u64>
    + AsMut<u64>
    + Add<u64>
    + AddAssign<u64>
    + Sub<u64>
    + SubAssign<u64>
    + Sub<Self>
    + Display
    + Debug
    + Default
{
    const NAME: &'static str;
    fn checked_add(self, rhs: u64) -> Option<Self>;
    fn checked_sub(self, rhs: u64) -> Option<Self>;
}

#[macro_export]
macro_rules! id_type {
    ($vis:vis $name:ident) => {
        #[derive(Clone, Default, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, $crate::serde::Serialize, $crate::serde::Deserialize)]
        #[repr(transparent)]
        #[serde(transparent)]
        $vis struct $name(u64);

        impl $name {
            pub const fn from_u64(id: u64) -> Self {
                Self(id)
            }

            pub const fn as_u64(self) -> u64 {
                self.0
            }
        }

        impl From<$name> for u64 {
            fn from(id: $name) -> u64 {
                id.0
            }
        }

        impl AsRef<u64> for $name {
            fn as_ref(&self) -> &u64 {
                &self.0
            }
        }

        impl AsMut<u64> for $name {
            fn as_mut(&mut self) -> &mut u64 {
                &mut self.0
            }
        }

        impl From<u64> for $name {
            fn from(id: u64) -> Self {
                Self(id)
            }
        }

        impl ::std::ops::Add<u64> for $name {
            type Output = Self;

            fn add(self, rhs: u64) -> Self {
                Self(self.0 + rhs)
            }
        }

        impl ::std::ops::AddAssign<u64> for $name {
            fn add_assign(&mut self, rhs: u64) {
                self.0 += rhs;
            }
        }

        impl ::std::ops::Sub<u64> for $name {
            type Output = Self;

            fn sub(self, rhs: u64) -> Self {
                Self(self.0 - rhs)
            }
        }

        impl ::std::ops::SubAssign<u64> for $name {
            fn sub_assign(&mut self, rhs: u64) {
                self.0 -= rhs;
            }
        }

        impl $crate::IdType for $name {
            const NAME: &'static str = stringify!($name);
            fn checked_add(self, rhs: u64) -> Option<Self> {
                self.0.checked_add(rhs).map(Self)
            }

            fn checked_sub(self, rhs: u64) -> Option<Self> {
                self.0.checked_sub(rhs).map(Self)
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }

        impl ::std::ops::Sub<$name> for $name {
            type Output = u64;

            fn sub(self, rhs: Self) -> u64 {
                self.0 - rhs.0
            }
        }

        impl $crate::hashbar::Hashbar for $name {
            fn hash<H: $crate::hashbar::Hasher>(&self, state: &mut H) {
                state.update(&self.0.to_le_bytes());
            }
        }
    };
}

id_type!(pub ClientId);
id_type!(pub RequestId);
id_type!(pub ReplicaId);

#[derive(Default)]
pub struct IdIter<I: IdType> {
    next: I,
}

impl<I: IdType> Iterator for IdIter<I> {
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        let ret = self.next;

        self.next = self.next.checked_add(1)?;

        Some(ret)
    }
}
