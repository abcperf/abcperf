extern crate sgx_types;

use core::num::NonZeroU32;
use std::{
    error::Error,
    fmt::{Debug, Display},
};

use sgx_types::error::SgxStatus;

pub mod slice;

mod sealed {
    use super::*;

    pub trait SgxStatusExt {}

    impl SgxStatusExt for SgxStatus {}
    impl<T> SgxStatusExt for Result<T, SgxStatus> {}

    pub trait SgxResultExt {}

    impl SgxResultExt for Result<(), SgxError> {}
}

pub trait SgxStatusExt: sealed::SgxStatusExt {
    type T;
    fn into_result(self) -> Result<Self::T, SgxError>;
}

impl SgxStatusExt for SgxStatus {
    type T = ();
    fn into_result(self) -> Result<Self::T, SgxError> {
        match SgxError::try_from(self) {
            Ok(error) => Err(error),
            Err(()) => Ok(()),
        }
    }
}

impl<T> SgxStatusExt for Result<T, SgxStatus> {
    type T = T;
    fn into_result(self) -> Result<Self::T, SgxError> {
        self.map_err(|e| e.try_into().expect("SgxStatus::Success in Result::Err"))
    }
}

pub trait SgxResultExt: sealed::SgxResultExt {
    fn into_status(self) -> SgxStatus;
}

impl SgxResultExt for Result<(), SgxError> {
    fn into_status(self) -> SgxStatus {
        match self {
            Ok(()) => SgxStatus::Success,
            Err(e) => e.into(),
        }
    }
}

pub type SgxResult<T = ()> = Result<T, SgxError>;

impl Display for SgxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SgxError")
            .field(&SgxStatus::from(*self).as_str());
        Ok(())
    }
}

impl Debug for SgxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self, f)
    }
}

impl Error for SgxError {}

#[repr(transparent)]
#[derive(Clone, Copy)]
/// SgxStatus without SGX_SUCCESS
pub struct SgxError {
    /// cant be SGX_SUCCESS
    inner: NonZeroU32,
}

impl SgxError {
    pub fn as_str(self) -> &'static str {
        SgxStatus::from(self).as_str()
    }
}

impl From<SgxError> for SgxStatus {
    fn from(error: SgxError) -> Self {
        SgxStatus::try_from(error.inner.get()).expect("always crated from SgxStatus")
    }
}

impl TryFrom<SgxStatus> for SgxError {
    type Error = ();

    fn try_from(status: SgxStatus) -> Result<Self, Self::Error> {
        if status.is_success() {
            Err(())
        } else {
            Ok(Self {
                inner: NonZeroU32::try_from(u32::from(status)).expect("its not success"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_result() {
        assert!(SgxStatus::Success.into_result().is_ok());

        for error in (0x0000_0001..=0x0000_8005).flat_map(SgxStatus::try_from) {
            assert_eq!(error, error.into_result().unwrap_err().into());
        }
    }

    #[test]
    fn fmt() {
        let error = SgxStatus::Unexpected.into_result().unwrap_err();
        let string = format!("{:?}", error);
        assert!(string.contains("SgxError"));
        assert!(string.contains("Unexpected"));
    }

    #[test]
    fn copy() {
        let error = SgxStatus::Unexpected.into_result().unwrap_err();
        #[allow(clippy::clone_on_copy)]
        let _ = error.clone();
    }
}
