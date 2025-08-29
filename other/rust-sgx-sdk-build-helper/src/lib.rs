use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use sgx_build_helper::try_run_suppressed;

mod deps;

const EDL_PATH: &str = "../../../rust-sgx-sdk/sgx_edl/edl";
const BUILD_LOCK_DIR_PATH: &str = "/tmp/abcperf_enclave_build.pid";
pub struct SgxSdk {
    name: &'static str,
    lib_path: PathBuf,
    include_path: PathBuf,
    edger8r_bin: PathBuf,
    mode: String,
    enclave_path: PathBuf,
}

impl SgxSdk {
    pub fn from_env(name: &'static str) -> Self {
        let sdk_path: PathBuf = env::var("SGX_SDK")
            .unwrap_or_else(|_| "/opt/sgxsdk".to_string())
            .parse()
            .unwrap();
        rerun_if_env_changed("SGX_SDK");

        let lib_path = sdk_path.join("lib64");
        let include_path = sdk_path.join("include");
        let bin_path = sdk_path.join("bin/x64");
        let edger8r_bin = bin_path.join("sgx_edger8r");

        let mode = env::var("SGX_MODE").unwrap_or_else(|_| "HW".to_string());
        rerun_if_env_changed("SGX_MODE");

        let enclave_path: PathBuf = format!("{}-enclave", env::var("CARGO_MANIFEST_DIR").unwrap(),)
            .parse()
            .unwrap();

        Self {
            name,
            lib_path,
            include_path,
            edger8r_bin,
            mode,
            enclave_path,
        }
    }

    pub fn build(&self) -> bool {
        self.acquire_build_lock();
        if !self.build_enclave() {
            Self::release_build_lock();
            return false;
        }

        if !self.gen_untrusted_lib() {
            Self::release_build_lock();
            return false;
        }
        Self::release_build_lock();
        true
    }

    fn acquire_build_lock(&self) {
        let build_start_time = Instant::now();
        while fs::create_dir(BUILD_LOCK_DIR_PATH).is_err() {
            if build_start_time.elapsed().as_secs() > 20 {
                let sleep_time = getrandom::u64().unwrap_or_else(|_| {
                    panic!("SGX Build Helper {}: Cannot get random number", self.name)
                }) % 45000;
                println!("SGX Build Helper {}: Another build is in progress, but it has been running for more than 60 seconds. Assuming it is stuck, sleeping for another {sleep_time} seconds, then continue.", self.name);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        ctrlc::set_handler(Self::release_build_lock).unwrap_or_else(|_| {
            panic!(
                "SGX Build Helper {}: Error setting Ctrl-C handler failed",
                self.name
            )
        });
    }

    fn release_build_lock() {
        let _ = fs::remove_dir(BUILD_LOCK_DIR_PATH);
    }

    fn build_enclave(&self) -> bool {
        rerun_if_changed(self.enclave_path.join("src"));
        rerun_if_changed(self.enclave_path.join("Cargo.toml"));
        rerun_if_changed(self.enclave_path.join("Makefile"));
        rerun_if_changed(self.enclave_path.join(format!("{}.edl", self.name)));
        rerun_if_changed(self.enclave_path.join(format!("{}.lds", self.name)));
        rerun_if_changed(self.enclave_path.join(format!("{}.config.xml", self.name)));
        rerun_if_changed(self.enclave_path.join(format!("{}_private.pem", self.name)));

        for dep in deps::get(&self.enclave_path) {
            rerun_if_changed(dep);
        }

        let filtered_env: HashMap<String, String> = env::vars()
            .filter(|(k, _)| {
                (!k.starts_with("CARGO")
                    && !k.starts_with("RUST")
                    && k != "PROFILE"
                    && k != "NUM_JOBS"
                    && k != "SSL_CERT_DIR"
                    && k != "SSL_CERT_FILE"
                    && k != "HOST"
                    && k != "OPT_LEVEL"
                    && k != "OUT_DIR"
                    && k != "DEBUG"
                    && k != "TARGET"
                    && k != "LD_LIBRARY_PATH")
                    || k == "CARGO_HOME"
                    || k == "RUSTUP_HOME"
                    || k == "RUST_VERSION"
            })
            .collect();

        try_run_suppressed(
            Command::new("make")
                .args(["-C", self.enclave_path.to_str().unwrap()])
                .env_clear()
                .envs(&filtered_env),
        )
    }

    fn gen_untrusted_lib(&self) -> bool {
        let out_dir: PathBuf = env::var("OUT_DIR").unwrap().parse().unwrap();
        let untrusted_lib_out = out_dir.join(format!("{}_untrusted_lib", self.name));
        fs::create_dir_all(&untrusted_lib_out).unwrap();

        let edl_file = self.enclave_path.join(format!("{}.edl", self.name));
        rerun_if_changed(&self.edger8r_bin);
        rerun_if_changed(&edl_file);
        if !try_run_suppressed(
            Command::new(&self.edger8r_bin)
                .arg("--use-prefix")
                .args(["--search-path", self.include_path.to_str().unwrap()])
                .args(["--search-path", EDL_PATH])
                .arg("--untrusted")
                .args(["--untrusted-dir", untrusted_lib_out.to_str().unwrap()])
                .arg(edl_file),
        ) {
            return false;
        }

        cc::Build::new()
            .file(
                untrusted_lib_out
                    .join(format!("{}_u.c", self.name))
                    .to_str()
                    .unwrap(),
            )
            .include(self.include_path.to_str().unwrap())
            .include(EDL_PATH)
            .compile(&format!("{}_u", self.name));
        true
    }

    pub fn link_lib(&self) {
        println!(
            "cargo:rustc-link-search=native={}",
            self.lib_path.to_str().unwrap()
        );

        match self.mode.as_ref() {
            "SIM" | "SW" => {
                println!("cargo:rustc-link-lib=dylib=sgx_urts_sim");
                println!("cargo:rustc-link-lib=dylib=sgx_epid_sim");
            }
            "HYPER" => {
                println!("cargo:rustc-link-lib=dylib=sgx_urts_hyper");
                unimplemented!();
            }
            _ => {
                println!("cargo:rustc-link-lib=dylib=sgx_urts");
                println!("cargo:rustc-link-lib=dylib=sgx_epid");
            }
        }
    }
}

fn rerun_if_changed(file: impl AsRef<Path>) {
    println!("cargo:rerun-if-changed={}", file.as_ref().to_str().unwrap());
}

fn rerun_if_env_changed(variable: &str) {
    println!("cargo:rerun-if-env-changed={}", variable);
}
