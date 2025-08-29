use rust_sgx_sdk_build_helper::SgxSdk;

fn main() {
    let sgx_sdk = SgxSdk::from_env("remote_attestation");
    sgx_sdk.link_lib();
}
