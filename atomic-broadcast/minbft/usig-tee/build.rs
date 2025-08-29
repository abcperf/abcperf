use rust_sgx_sdk_build_helper::SgxSdk;

fn main() {
    let sgx_sdk = SgxSdk::from_env("usig_tee");
    if sgx_sdk.build() {
        sgx_sdk.link_lib();
    } else {
        panic!("Failed to build usig_tee");
    }
}
