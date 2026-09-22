# ==============================================================================
# LG Hue Sync — Build & Deployment Automation
# Target: LG C1 OLED (webOS 6.x / armv7l / glibc 2.28)
# ==============================================================================

SHELL := /bin/bash
.SHELLFLAGS := -euo pipefail -c
.DEFAULT_GOAL := help

# --- Configuration & Defaults -------------------------------------------------
TV_IP          ?= 192.168.1.149
SSH_PORT       ?= 22
HUE_BRIDGE_IP  ?= 192.168.1.151
TARGET         := armv7-unknown-linux-gnueabi
BINARY         := target/$(TARGET)/release/lg-hue-sync
REMOTE_DIR     := /var/home/root/lg-hue-sync

SSH_OPTS       := -p $(SSH_PORT) -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5
SCP_OPTS       := -P $(SSH_PORT) -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5

# Colors for terminal output
BOLD   := \033[1m
GREEN  := \033[32m
CYAN   := \033[36m
YELLOW := \033[33m
RED    := \033[31m
RESET  := \033[0m

.PHONY: help build build-local test test-pattern deploy deploy-bin deploy-config deploy-app \
        provision-luna status logs start stop restart test-capture ssh root pair clean

## display available targets and usage
help:
	@printf "$(BOLD)LG Hue Sync — Automation Commands$(RESET)\n\n"
	@printf "$(CYAN)Build Targets:$(RESET)\n"
	@printf "  $(GREEN)make build$(RESET)         Cross-compile release binary for LG webOS (ARMv7, Debian Buster container)\n"
	@printf "  $(GREEN)make build-local$(RESET)   Build binary for host OS (macOS) via local cargo\n"
	@printf "  $(GREEN)make clean$(RESET)         Clean build artifacts\n\n"
	@printf "$(CYAN)Testing & Validation:$(RESET)\n"
	@printf "  $(GREEN)make test$(RESET)          Run local Rust unit and integration tests\n"
	@printf "  $(GREEN)make test-pattern$(RESET)  Run local rainbow test pattern across Hue and Nanoleaf (Mac -> Lights)\n"
	@printf "  $(GREEN)make test-capture$(RESET)  Run vtcapture HDMI screen capture probe directly on TV over SSH\n\n"
	@printf "$(CYAN)Deployment (TV IP: $(TV_IP)):$(RESET)\n"
	@printf "  $(GREEN)make deploy$(RESET)        Full deployment: build, upload binary & config, configure systemd/init.d, start\n"
	@printf "  $(GREEN)make deploy-bin$(RESET)    Quick deploy: scp binary only to TV and restart daemon\n"
	@printf "  $(GREEN)make deploy-config$(RESET) Quick deploy: scp config.json only to TV and restart daemon\n"
	@printf "  $(GREEN)make provision-luna$(RESET) Provision Luna manifests and permissions for vtcapture on TV\n\n"
	@printf "$(CYAN)Service Management & Remote Shell:$(RESET)\n"
	@printf "  $(GREEN)make status$(RESET)        Check daemon systemd status on TV\n"
	@printf "  $(GREEN)make logs$(RESET)          Follow daemon logs on TV (journalctl -f)\n"
	@printf "  $(GREEN)make start$(RESET)         Start daemon on TV\n"
	@printf "  $(GREEN)make stop$(RESET)          Stop daemon on TV\n"
	@printf "  $(GREEN)make restart$(RESET)       Restart daemon on TV\n"
	@printf "  $(GREEN)make ssh$(RESET)           Open interactive root shell on TV\n\n"
	@printf "$(CYAN)Setup & Tooling:$(RESET)\n"
	@printf "  $(GREEN)make root$(RESET)          Execute SlopBro network autoroot on TV (if SSH is closed)\n"
	@printf "  $(GREEN)make pair$(RESET)          Pair Hue Bridge and discover entertainment zones\n\n"

## cross-compile ARMv7 binary matching webOS 6.x glibc 2.28 in Docker container
build:
	@printf "$(CYAN)[*] Cross-compiling for $(TARGET) in Debian Buster container...$(RESET)\n"
	mkdir -p /tmp/docker_root/.cargo /tmp/docker_root/.rustup
	docker run --rm \
	  -v "$$PWD":/app -w /app \
	  -v /tmp/docker_root/.cargo:/root/.cargo \
	  -v /tmp/docker_root/.rustup:/root/.rustup \
	  debian:buster bash -c '\
	    set -e; \
	    echo "deb [trusted=yes] http://archive.debian.org/debian buster main" > /etc/apt/sources.list; \
	    echo "deb [trusted=yes] http://archive.debian.org/debian-security buster/updates main" >> /etc/apt/sources.list; \
	    apt-get -o Acquire::Check-Valid-Until=false update -qq; \
	    apt-get install --allow-unauthenticated -y -qq build-essential gcc-arm-linux-gnueabi libc6-dev-armel-cross binutils-arm-linux-gnueabi make perl curl ca-certificates > /dev/null; \
	    if ! command -v rustup &> /dev/null; then \
	      curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal > /dev/null; \
	    fi; \
	    source /root/.cargo/env; \
	    rustup target add $(TARGET); \
	    export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER=arm-linux-gnueabi-gcc; \
	    export CC_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-gcc; \
	    export AR_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-ar; \
	    export RANLIB_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-ranlib; \
	    export CARGO_TARGET_DIR=/tmp/target; \
	    cargo build --target $(TARGET) --release; \
	    mkdir -p /app/target/$(TARGET)/release; \
	    cp /tmp/target/$(TARGET)/release/lg-hue-sync /app/target/$(TARGET)/release/lg-hue-sync; \
	  '
	@printf "$(GREEN)[+] Build complete: $(BINARY) ($$(du -h $(BINARY) | cut -f1))$(RESET)\n"

## build binary locally on host machine
build-local:
	@printf "$(CYAN)[*] Building host binary with cargo...$(RESET)\n"
	cargo build --release
	@printf "$(GREEN)[+] Host build complete: target/release/lg-hue-sync$(RESET)\n"

## run local unit tests
test:
	@printf "$(CYAN)[*] Running test suite...$(RESET)\n"
	cargo test

## run live test pattern locally from Mac against Hue and Nanoleaf
test-pattern:
	@printf "$(CYAN)[*] Running rainbow test pattern on local Mac...$(RESET)\n"
	cargo run -- test-pattern --config config.json

## full deployment: build if needed, configure TV, deploy binary & config, restart
deploy:
	@if [ ! -f "$(BINARY)" ]; then \
	  $(MAKE) build; \
	fi
	@./scripts/deploy.sh $(TV_IP) $(SSH_PORT)

## quick deploy: upload binary only to TV, provision Luna, and restart daemon
deploy-bin: provision-luna
	@if [ ! -f "$(BINARY)" ]; then \
	  printf "$(RED)[-] Binary $(BINARY) not found. Run 'make build' first.$(RESET)\n"; \
	  exit 1; \
	fi
	@printf "$(CYAN)[*] Deploying binary to root@$(TV_IP)...$(RESET)\n"
	ssh $(SSH_OPTS) root@$(TV_IP) "systemctl stop lg-hue-sync 2>/dev/null || true; mkdir -p $(REMOTE_DIR)"
	sleep 2
	scp $(SCP_OPTS) $(BINARY) root@$(TV_IP):$(REMOTE_DIR)/lg-hue-sync.new
	ssh $(SSH_OPTS) root@$(TV_IP) "mv -f $(REMOTE_DIR)/lg-hue-sync.new $(REMOTE_DIR)/lg-hue-sync && chmod +x $(REMOTE_DIR)/lg-hue-sync && systemctl start lg-hue-sync"
	@printf "$(GREEN)[+] Binary deployed and service started.$(RESET)\n"

## provision Luna Service 2 manifests and permissions on TV
provision-luna:
	@./scripts/provision_luna.sh $(TV_IP) $(SSH_PORT)

## quick deploy: upload config.json only to TV and restart daemon
deploy-config:
	@if [ ! -f "config.json" ]; then \
	  printf "$(RED)[-] config.json not found.$(RESET)\n"; \
	  exit 1; \
	fi
	@printf "$(CYAN)[*] Deploying config.json to root@$(TV_IP)...$(RESET)\n"
	ssh $(SSH_OPTS) root@$(TV_IP) "mkdir -p $(REMOTE_DIR)"
	scp $(SCP_OPTS) config.json root@$(TV_IP):$(REMOTE_DIR)/config.json
	ssh $(SSH_OPTS) root@$(TV_IP) "systemctl restart lg-hue-sync"
	@printf "$(GREEN)[+] Config deployed and service restarted.$(RESET)\n"

## package and deploy webOS application to Home Dashboard ribbon
deploy-app:
	@printf "$(CYAN)[*] Packaging webOS application...$(RESET)\n"
	uv run scripts/package_ipk.py
	@printf "$(CYAN)[*] Installing application on TV...$(RESET)\n"
	scp $(SCP_OPTS) target/org.webosbrew.lg-hue-sync_0.3.0_all.ipk root@$(TV_IP):/tmp/org.webosbrew.lg-hue-sync.ipk
	ssh $(SSH_OPTS) root@$(TV_IP) "luna-send -n 1 -f luna://com.webos.appInstallService/dev/install '{\"id\":\"org.webosbrew.lg-hue-sync\", \"ipkUrl\":\"/tmp/org.webosbrew.lg-hue-sync.ipk\", \"subscribe\":false}'"
	@printf "$(GREEN)[+] Application installed on TV Home Dashboard.$(RESET)\n"

## run vtcapture screen capture probe on TV over SSH
test-capture:
	@printf "$(CYAN)[*] Running test-capture on TV (160x90 vtcapture HDMI probe)...$(RESET)\n"
	ssh -t $(SSH_OPTS) root@$(TV_IP) "$(REMOTE_DIR)/lg-hue-sync test-capture --config $(REMOTE_DIR)/config.json"

## check daemon systemd service status on TV
status:
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl status lg-hue-sync --no-pager -l || true"

## follow live logs from TV daemon
logs:
	@ssh -t $(SSH_OPTS) root@$(TV_IP) "touch $(REMOTE_DIR)/daemon.log && tail -f -n 50 $(REMOTE_DIR)/daemon.log"

## start daemon service on TV
start:
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl start lg-hue-sync"
	@printf "$(GREEN)[+] lg-hue-sync started.$(RESET)\n"

## stop daemon service on TV
stop:
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl stop lg-hue-sync"
	@printf "$(YELLOW)[+] lg-hue-sync stopped.$(RESET)\n"

## restart daemon service on TV
restart:
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl stop lg-hue-sync 2>/dev/null || true; sleep 2; systemctl start lg-hue-sync"
	@printf "$(GREEN)[+] lg-hue-sync restarted.$(RESET)\n"

## open interactive root SSH session to TV
ssh:
	@ssh -t $(SSH_OPTS) root@$(TV_IP)

## autoroot TV over LAN using SlopBro (webOS 6.x)
root:
	uv run scripts/root_tv.py --webos-version 6 $(TV_IP)

## pair Hue Bridge and discover entertainment zones
pair:
	uv run scripts/pair_hue.py --tv-ip $(TV_IP)

## clean cargo target directory
clean:
	cargo clean
