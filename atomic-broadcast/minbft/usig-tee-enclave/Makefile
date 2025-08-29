# Licensed to the Apache Software Foundation (ASF) under one
# or more contributor license agreements.  See the NOTICE file
# distributed with this work for additional information
# regarding copyright ownership.  The ASF licenses this file
# to you under the Apache License, Version 2.0 (the
# "License"); you may not use this file except in compliance
# with the License.  You may obtain a copy of the License at
#
#   http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing,
# software distributed under the License is distributed on an
# "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
# KIND, either express or implied.  See the License for the
# specific language governing permissions and limitations
# under the License.

######## SGX SDK Settings ########

SGX_SDK ?= /opt/intel/sgxsdk
SGX_MODE ?= HW
SGX_ARCH ?= x64

TOP_DIR := ../../../rust-sgx-sdk
include $(TOP_DIR)/buildenv.mk

ifeq ($(shell getconf LONG_BIT), 32)
	SGX_ARCH := x86
else ifeq ($(findstring -m32, $(CXXFLAGS)), -m32)
	SGX_ARCH := x86
endif

ifeq ($(SGX_ARCH), x86)
	SGX_COMMON_CFLAGS := -m32
	SGX_BIN_PATH := $(SGX_SDK)/bin/x86
else
	SGX_COMMON_CFLAGS := -m64
	SGX_BIN_PATH := $(SGX_SDK)/bin/x64
endif

ifeq ($(SGX_DEBUG), 1)
	SGX_COMMON_CFLAGS += -O0 -g
	Rust_Build_Flags :=
	Rust_Build_Out := debug
else
	SGX_COMMON_CFLAGS += -O2
	Rust_Build_Flags := --release
	Rust_Build_Out := release
endif

SGX_EDGER8R := $(SGX_BIN_PATH)/sgx_edger8r
ifneq ($(SGX_MODE), HYPER)
	SGX_ENCLAVE_SIGNER := $(SGX_BIN_PATH)/sgx_sign
else
	SGX_ENCLAVE_SIGNER := $(SGX_BIN_PATH)/sgx_sign_hyper
	SGX_EDGER8R_MODE := --sgx-mode $(SGX_MODE)
endif

######## CUSTOM Settings ########

RUST_ENCLAVE_NAME := usig_tee
CUSTOM_SYSROOT_PATH := ./sysroot
CUSTOM_EDL_PATH := $(ROOT_DIR)/sgx_edl/edl
CUSTOM_COMMON_PATH := $(ROOT_DIR)/common

######## EDL Settings ########

Enclave_EDL_Files := $(RUST_ENCLAVE_NAME)_t.c $(RUST_ENCLAVE_NAME)_t.h

######## Enclave Settings ########

# BUILD_STD=no       use no_std
# BUILD_STD=cargo    use cargo-std-aware
# BUILD_STD=xargo    use xargo
BUILD_STD ?= cargo

Rust_Build_Target := x86_64-unknown-linux-sgx
Rust_Target_Path := $(ROOT_DIR)/rustlib

ifneq ($(BUILD_STD), cargo)
ifneq ($(BUILD_STD), xargo)
$(error Only supports building with build_std strategy!!)
endif
endif

export CARGO_FEATURE_STD=1
ifeq ($(BUILD_STD), cargo)
	Rust_Build_Std := $(Rust_Build_Flags) -Zbuild-std=core,alloc
	Rust_Std_Features := --features net,thread,untrusted_time,untrusted_fs,unsupported_process
	Rust_Target_Flags := --target $(Rust_Target_Path)/$(Rust_Build_Target).json
	Rust_Sysroot_Path := $(CURDIR)/sysroot
	Rust_Sysroot_Flags := RUSTFLAGS="--sysroot $(Rust_Sysroot_Path)"
else
	Rust_Unstable_Flags := RUSTFLAGS="-Z force-unstable-if-unmarked"
endif

RustEnclave_Build_Flags := $(Rust_Build_Flags)
RustEnclave_Include_Paths := -I$(CUSTOM_COMMON_PATH)/inc -I$(CUSTOM_COMMON_PATH)/inc/tlibc -I$(CUSTOM_EDL_PATH)

RustEnclave_Link_Libs := -Ltarget/$(Rust_Build_Target)/$(Rust_Build_Out) -l$(RUST_ENCLAVE_NAME)_enclave
RustEnclave_C_Flags := $(CFLAGS) $(ENCLAVE_CFLAGS) $(SGX_COMMON_CFLAGS) $(RustEnclave_Include_Paths)
RustEnclave_Link_Flags := -Wl,--no-undefined -nostdlib -nodefaultlibs -nostartfiles \
	-Wl,--start-group $(RustEnclave_Link_Libs) -Wl,--end-group \
	-Wl,--version-script=$(RUST_ENCLAVE_NAME).lds \
	$(ENCLAVE_LDFLAGS)

RustEnclave_Name := $(RUST_ENCLAVE_NAME).so
RustEnclave_Signed_Name := $(RUST_ENCLAVE_NAME).signed.so
RustEnclave_Edl_Name := $(RUST_ENCLAVE_NAME).edl

.PHONY: all
all: $(Enclave_EDL_Files) $(RustEnclave_Signed_Name)

######## EDL Objects ########

$(Enclave_EDL_Files): $(SGX_EDGER8R) $(RustEnclave_Edl_Name)
	$(SGX_EDGER8R) $(SGX_EDGER8R_MODE) --use-prefix --trusted $(RustEnclave_Edl_Name) --search-path $(CUSTOM_COMMON_PATH)/inc --search-path $(CUSTOM_EDL_PATH) --trusted-dir .
	@echo "GEN => $(Enclave_EDL_Files)"

######## Enclave Objects ########

$(RUST_ENCLAVE_NAME)_t.o: $(Enclave_EDL_Files)
	@$(CC) $(RustEnclave_C_Flags) -c $(RUST_ENCLAVE_NAME)_t.c -o $@

$(RustEnclave_Name): $(RUST_ENCLAVE_NAME)_t.o enclave
	@$(CXX) $(RUST_ENCLAVE_NAME)_t.o -o $@ $(RustEnclave_Link_Flags)
	@echo "LINK => $@"

$(RustEnclave_Signed_Name): $(RustEnclave_Name) $(RUST_ENCLAVE_NAME).config.xml
	@$(SGX_ENCLAVE_SIGNER) sign -key $(RUST_ENCLAVE_NAME)_private.pem -enclave $(RustEnclave_Name) -out $@ -config $(RUST_ENCLAVE_NAME).config.xml
	@echo "SIGN => $@"

######## Build Enclave ########

.PHONY: enclave
enclave:
ifeq ($(BUILD_STD), cargo)
	@cd $(Rust_Target_Path)/std && cargo build $(Rust_Build_Std) $(Rust_Target_Flags) $(Rust_Std_Features)

	@rm -rf $(Rust_Sysroot_Path)
	@mkdir -p $(Rust_Sysroot_Path)/lib/rustlib/$(Rust_Build_Target)/lib
	@cp -r $(Rust_Target_Path)/std/target/$(Rust_Build_Target)/$(Rust_Build_Out)/deps/* $(Rust_Sysroot_Path)/lib/rustlib/$(Rust_Build_Target)/lib

	@cd $(Rust_Target_Path)/proc_macro && $(Rust_Sysroot_Flags) cargo build $(Rust_Target_Flags) $(RustEnclave_Build_Flags)
	@sleep 1
	@cp -r $(Rust_Target_Path)/proc_macro/target/$(Rust_Build_Target)/$(Rust_Build_Out)/deps/* $(Rust_Sysroot_Path)/lib/rustlib/$(Rust_Build_Target)/lib

	@$(Rust_Sysroot_Flags) cargo build $(Rust_Target_Flags) $(RustEnclave_Build_Flags)
else
	@$(Rust_Unstable_Flags) RUST_TARGET_PATH=$(Rust_Target_Path) xargo build --target $(Rust_Build_Target) $(RustEnclave_Build_Flags)
endif

.PHONY: clean
clean:
	@rm -f $(RustEnclave_Name) $(RustEnclave_Signed_Name) *_t.*
	@cargo clean
	@cd $(Rust_Target_Path)/std && cargo clean
	@cd $(Rust_Target_Path)/proc_macro && cargo clean
	@rm -rf $(CUSTOM_SYSROOT_PATH)
