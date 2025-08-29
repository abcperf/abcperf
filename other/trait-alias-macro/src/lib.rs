#[doc(hidden)]
#[macro_export]
macro_rules! do_not_use {
    ($($item:item)*) => ($($item)*);
}

#[macro_export]
macro_rules! trait_alias_macro {
    ($name:ident = $($base:tt)+) => {
        ::trait_alias_macro::do_not_use! {
            trait $name: $($base)+ { }
            impl<T: $($base)+> $name for T { }
        }
    };
}

#[macro_export]
macro_rules! pub_trait_alias_macro {
    ($name:ident = $($base:tt)+) => {
        ::trait_alias_macro::do_not_use! {
            pub trait $name: $($base)+ { }
            impl<T: $($base)+> $name for T { }
        }
    };
}
