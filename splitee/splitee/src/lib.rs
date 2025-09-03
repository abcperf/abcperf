#[derive(Debug, Clone, Default)]
pub struct SpliTee<S>(pub S);

pub trait SpliTeeCrypt {}

pub trait SpliTeeSign {}

pub trait SpliTeeCoin {}

impl<S: SpliTeeCrypt> SpliTee<S> {}

impl<S: SpliTeeSign> SpliTee<S> {}

impl<S: SpliTeeCoin> SpliTee<S> {}
