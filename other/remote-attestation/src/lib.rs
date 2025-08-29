use once_cell::sync::Lazy;

use std::io::Write;
use std::path::PathBuf;

#[doc(hidden)]
pub type EnclavePathType = Lazy<PathBuf>;

#[doc(hidden)]
pub fn init_enclave(bin: &[u8]) -> PathBuf {
    let path = format!("/tmp/{}.so", ::uuid::Uuid::new_v4()).into();
    let mut file = ::std::fs::File::create(&path).unwrap();
    file.write_all(bin).unwrap();
    drop(file);
    path
}

#[macro_export]
macro_rules! include_enclave {
    ($enclave:expr) => {
        static ENCLAVE_PATH: ::remote_attestation::EnclavePathType =
            ::remote_attestation::EnclavePathType::new(|| {
                ::remote_attestation::init_enclave(include_bytes!($enclave))
            });
    };
}
