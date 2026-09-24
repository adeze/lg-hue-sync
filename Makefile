# ==============================================================================
# LG Hue Sync — Build & Deployment Automation
# Target: LG C1 OLED (webOS 6.x / armv7l / glibc 2.28)
# ==============================================================================

SHELL := /bin/bash
.SHELLFLAGS := -euo pipefail -c
.DEFAULT_GOAL := help

# --- Configuration & Defaults -------------------------------------------------
TV_IP          ?=
SSH_PORT       ?= 22
TARGET         := armv7-unknown-linux-gnueabi
BINARY         := target/$(TARGET)/release/lg-hue-sync
REMOTE_DIR     := /var/home/root/lg-hue-sync
RUST_VERSION   ?= 1.98.1
CROSS_IMAGE    ?= lg-hue-sync-cross:rust-$(RUST_VERSION)
CARGO_CACHE    ?= lg-hue-sync-cargo
TARGET_CACHE   ?= lg-hue-sync-target

SSH_OPTS       := -p $(SSH_PORT) -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5
SCP_OPTS       := -P $(SSH_PORT) -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5

# Colors for terminal output
BOLD   := \033[1m
GREEN  := \033[32m
CYAN   := \033[36m
YELLOW := \033[33m
RED    := \033[31m
RESET  := \033[0m

.PHONY: help setup check require-tv cross-image build build-local test test-pattern deps-check deps-update release-check \
        deploy deploy-bin deploy-config deploy-app provision-luna status logs start stop restart \
        test-capture ssh root pair clean cross-clean

## display available targets and usage
help:
	@printf "$(BOLD)LG Hue Sync — Automation Commands$(RESET)\n\n"
	@printf "$(CYAN)Build Targets:$(RESET)\n"
	@printf "  $(GREEN)make cross-image$(RESET)   Build the cached Debian Buster/Rust cross-toolchain image\n"
	@printf "  $(GREEN)make build$(RESET)         Cross-compile release binary for LG webOS (ARMv7, Debian Buster container)\n"
	@printf "  $(GREEN)make build-local$(RESET)   Build binary for host OS (macOS) via local cargo\n"
	@printf "  $(GREEN)make clean$(RESET)         Clean host build artifacts\n"
	@printf "  $(GREEN)make cross-clean$(RESET)   Remove cross-build image and Docker caches\n\n"
	@printf "$(CYAN)Testing & Validation:$(RESET)\n"
	@printf "  $(GREEN)make setup$(RESET)         Install host Rust components and fetch locked dependencies\n"
	@printf "  $(GREEN)make check$(RESET)         Run the complete host validation gate used by CI\n"
	@printf "  $(GREEN)make test$(RESET)          Run local Rust unit and integration tests\n"
	@printf "  $(GREEN)make deps-check$(RESET)    Report available compatible dependency updates\n"
	@printf "  $(GREEN)make deps-update$(RESET)   Update Cargo.lock within Cargo.toml constraints\n"
	@printf "  $(GREEN)make release-check$(RESET) Validate version metadata before tagging\n"
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

## build the reusable ARMv7/glibc 2.28 cross-toolchain image
cross-image:
	docker build --build-arg RUST_VERSION=$(RUST_VERSION) -t $(CROSS_IMAGE) -f docker/Dockerfile.cross docker

## prepare a local/Codex worktree without building the Docker target image
setup:
	rustup component add rustfmt clippy
	cargo fetch --locked

## run the host validation gate used by CI
check: release-check
	cargo fmt --all -- --check
	cargo test
	cargo clippy --bin lg-hue-sync -- -D warnings
	perl -0777 -ne 'print $$1 if /<script>(.*)<\/script>/s' src/web/ui.html | node --check -
	git diff --check

## cross-compile ARMv7 binary matching webOS 6.x glibc 2.28
build: cross-image
	@printf "$(CYAN)[*] Cross-compiling for $(TARGET) in Debian Buster container...$(RESET)\n"
	docker run --rm \
	  -v "$$PWD":/app -w /app \
	  -v $(CARGO_CACHE):/cargo-cache \
	  -v $(TARGET_CACHE):/target-cache \
	  $(CROSS_IMAGE)
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

deps-check:
	cargo update --dry-run
	cargo tree --duplicates

deps-update:
	cargo update

release-check:
	python3 scripts/check_release.py

## run live test pattern locally from Mac against Hue and Nanoleaf
test-pattern:
	@printf "$(CYAN)[*] Running rainbow test pattern on local Mac...$(RESET)\n"
	cargo run -- test-pattern --config config.json

## full deployment: build if needed, configure TV, deploy binary & config, restart
require-tv:
	@test -n "$(TV_IP)" || { echo "TV_IP is required, for example: make status TV_IP=192.0.2.10" >&2; exit 2; }

deploy: require-tv
	@if [ ! -f "$(BINARY)" ]; then \
	  $(MAKE) build; \
	fi
	@./scripts/deploy.sh $(TV_IP) $(SSH_PORT)

## quick deploy: upload binary only to TV, provision Luna, and restart daemon
deploy-bin: require-tv provision-luna
	@if [ ! -f "$(BINARY)" ]; then \
	  printf "$(RED)[-] Binary $(BINARY) not found. Run 'make build' first.$(RESET)\n"; \
	  exit 1; \
	fi
	@printf "$(CYAN)[*] Deploying binary to root@$(TV_IP)...$(RESET)\n"
	ssh $(SSH_OPTS) root@$(TV_IP) "mkdir -p $(REMOTE_DIR)"
	scp $(SCP_OPTS) $(BINARY) root@$(TV_IP):$(REMOTE_DIR)/lg-hue-sync.new
	@LOCAL_SHA=$$(shasum -a 256 $(BINARY) | awk '{print $$1}'); \
	REMOTE_SHA=$$(ssh $(SSH_OPTS) root@$(TV_IP) "sha256sum $(REMOTE_DIR)/lg-hue-sync.new" | awk '{print $$1}'); \
	test "$$LOCAL_SHA" = "$$REMOTE_SHA" || { echo "Binary checksum mismatch" >&2; exit 1; }
	ssh $(SSH_OPTS) root@$(TV_IP) "systemctl stop lg-hue-sync 2>/dev/null || true; if [ -x $(REMOTE_DIR)/lg-hue-sync ]; then cp -f $(REMOTE_DIR)/lg-hue-sync $(REMOTE_DIR)/lg-hue-sync.previous; fi; mv -f $(REMOTE_DIR)/lg-hue-sync.new $(REMOTE_DIR)/lg-hue-sync; chmod +x $(REMOTE_DIR)/lg-hue-sync; systemctl start lg-hue-sync"
	@printf "$(GREEN)[+] Binary deployed and service started.$(RESET)\n"

## provision Luna Service 2 manifests and permissions on TV
provision-luna: require-tv
	@./scripts/provision_luna.sh $(TV_IP) $(SSH_PORT)

## quick deploy: upload config.json only to TV and restart daemon
deploy-config: require-tv
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
deploy-app: require-tv
	@printf "$(CYAN)[*] Packaging webOS application...$(RESET)\n"
	uv run scripts/package_ipk.py
	@printf "$(CYAN)[*] Installing application on TV...$(RESET)\n"
	IPK_PATH=$$(find target -maxdepth 1 -name 'org.webosbrew.lg-hue-sync_*_all.ipk' -type f | sort | tail -1); test -n "$$IPK_PATH"; scp $(SCP_OPTS) "$$IPK_PATH" root@$(TV_IP):/tmp/org.webosbrew.lg-hue-sync.ipk
	ssh $(SSH_OPTS) root@$(TV_IP) "luna-send -n 1 -f luna://com.webos.appInstallService/dev/install '{\"id\":\"org.webosbrew.lg-hue-sync\", \"ipkUrl\":\"/tmp/org.webosbrew.lg-hue-sync.ipk\", \"subscribe\":false}'"
	@printf "$(GREEN)[+] Application installed on TV Home Dashboard.$(RESET)\n"

## run vtcapture screen capture probe on TV over SSH
test-capture: require-tv
	@printf "$(CYAN)[*] Running test-capture on TV (160x90 vtcapture HDMI probe)...$(RESET)\n"
	ssh -t $(SSH_OPTS) root@$(TV_IP) "$(REMOTE_DIR)/lg-hue-sync test-capture --config $(REMOTE_DIR)/config.json"

## check daemon systemd service status on TV
status: require-tv
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl status lg-hue-sync --no-pager -l || true"

## follow live logs from TV daemon
logs: require-tv
	@ssh -t $(SSH_OPTS) root@$(TV_IP) "touch $(REMOTE_DIR)/daemon.log && tail -f -n 50 $(REMOTE_DIR)/daemon.log"

## start daemon service on TV
start: require-tv
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl start lg-hue-sync"
	@printf "$(GREEN)[+] lg-hue-sync started.$(RESET)\n"

## stop daemon service on TV
stop: require-tv
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl stop lg-hue-sync"
	@printf "$(YELLOW)[+] lg-hue-sync stopped.$(RESET)\n"

## restart daemon service on TV
restart: require-tv
	@ssh $(SSH_OPTS) root@$(TV_IP) "systemctl stop lg-hue-sync 2>/dev/null || true; sleep 2; systemctl start lg-hue-sync"
	@printf "$(GREEN)[+] lg-hue-sync restarted.$(RESET)\n"

## open interactive root SSH session to TV
ssh: require-tv
	@ssh -t $(SSH_OPTS) root@$(TV_IP)

## autoroot TV over LAN using SlopBro (webOS 6.x)
root: require-tv
	uv run scripts/root_tv.py --webos-version 6 $(TV_IP)

## pair Hue Bridge and discover entertainment zones
pair: require-tv
	uv run scripts/pair_hue.py --tv-ip $(TV_IP)

## clean cargo target directory
clean:
	cargo clean

cross-clean:
	-docker image rm $(CROSS_IMAGE)
	-docker volume rm $(CARGO_CACHE) $(TARGET_CACHE)
