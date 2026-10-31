# Bootstrap needs only Rust, a native C toolchain and make; no game assets required.
CARGO ?= cargo
JOBS ?= 2
TARGET_DIR ?= target
DATA ?=
LOCALE ?= enUS
SERVER ?= 127.0.0.1:3724
MANAGER = $(TARGET_DIR)/release/wrathplus-manager$(EXE)
ifeq ($(OS),Windows_NT)
EXE = .exe
endif

.PHONY: help launcher manager run doctor setup all
.DEFAULT_GOAL := launcher

help:
	@printf '%s\n' 'make launcher   Build the Manager only (no game assets needed)' 'make manager    Build and open the Manager (alias: make run)' 'make doctor     Check game-build prerequisites without compiling the game' 'make setup DATA="/path/to/Data" [LOCALE=enUS SERVER=host:3724]' '                Validate your own assets and build the playable client, without launching' 'make all DATA=… Same complete setup; no server build or asset download'

launcher:
	@command -v "$(CARGO)" >/dev/null 2>&1 || { printf '%s\n' 'Cargo not found. Install Rust using rustup: https://rustup.rs (see docs/CLIENT-SETUP.md)'; exit 1; }
	$(CARGO) build --locked -p wrathplus-manager --release --jobs $(JOBS) --target-dir "$(TARGET_DIR)"

manager: launcher
	"$(MANAGER)" --repo "$(CURDIR)" --target-dir "$(abspath $(TARGET_DIR))"

setup: launcher
	@test -n "$(DATA)" || (printf '%s\n' 'Set DATA to the Data folder of your legally obtained 3.3.5a/build 12340 install.'; exit 2)
	"$(MANAGER)" --setup --repo "$(CURDIR)" --target-dir "$(abspath $(TARGET_DIR))" --data "$(DATA)" --locale "$(LOCALE)" --server "$(SERVER)"

run: manager

doctor: launcher
	"$(MANAGER)" --doctor --repo "$(CURDIR)" --target-dir "$(abspath $(TARGET_DIR))"

all: setup
