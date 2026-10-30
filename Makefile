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
RUST_VERSION   ?= stable
CROSS_IMAGE    ?= lg-hue-sync-cross:rust-$(RUST_VERSION)
CARGO_CACHE    ?= lg-hue-sync-cargo
TARGET_CACHE   ?= lg-hue-sync-target
WEBOS_NATIVE_HOME ?= $(HOME)/.local/share/webosbrew-rust
ARES_DEVICE    ?=
APP_ID         := org.webosbrew.lg-hue-sync

SSH_OPTS       := -p $(SSH_PORT) -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5
SCP_OPTS       := -P $(SSH_PORT) -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5

# Colors for terminal output
BOLD   := \033[1m
GREEN  := \033[32m
CYAN   := \033[36m
YELLOW := \033[33m
RED    := \033[31m
RESET  := \033[0m

.PHONY: help setup check toolchain-update docker-toolchain-update require-tv cross-image build build-docker native-setup build-native package-app require-ares-device install-app launch-app list-apps build-local test test-pattern deps-check deps-update release-check \
        deploy deploy-bin deploy-config deploy-app provision-luna status logs start stop restart \
        test-capture ssh root pair clean cross-clean

## display available targets and usage
help:
	@printf "$(BOLD)LG Hue Sync — Automation Commands$(RESET)\n\n"
	@printf "$(CYAN)Build Targets:$(RESET)\n"
	@printf "  $(GREEN)make cross-image$(RESET)   Build the cached Debian Buster/Rust cross-toolchain image\n"
	@printf "  $(GREEN)make toolchain-update$(RESET) Refresh stable Rust, check and build natively\n"
	@printf "  $(GREEN)make build$(RESET)         Cross-compile ARMv7 release on Mac using webOS SDK/Buster sysroot\n"
	@printf "  $(GREEN)make build-docker$(RESET)  Explicit cached Docker fallback (also used by Linux CI)\n"
	@printf "  $(GREEN)make docker-toolchain-update$(RESET) Rebuild fallback image with current stable Rust\n"
	@printf "  $(GREEN)make build-local$(RESET)   Build binary for host OS (macOS) via local cargo\n"
	@printf "  $(GREEN)make native-setup$(RESET)  Prepare Mac SDK/Buster sysroot from cached builder once\n"
	@printf "  $(GREEN)make build-native$(RESET)  Build verified ARMv7 binary on Mac without Docker\n"
	@printf "  $(GREEN)make package-app$(RESET)   Package launcher locally using Rust Ares\n"
	@printf "  $(GREEN)make install-app/launch-app/list-apps ARES_DEVICE=<name>$(RESET) Manage launcher via Rust Ares\n"
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

## refresh stable Rust and verify the default native build
toolchain-update:
	rustup update stable
	$(MAKE) check build

## explicitly refresh the cached Docker fallback
docker-toolchain-update:
	docker build --pull --no-cache --build-arg RUST_VERSION=$(RUST_VERSION) -t $(CROSS_IMAGE) -f docker/Dockerfile.cross docker
	docker run --rm $(CROSS_IMAGE) rustc --version
	$(MAKE) check build-docker

## prepare a local/Codex worktree without building the Docker target image
setup:
	rustup component add rustfmt clippy
	cargo fetch --locked

## run the host validation gate used by CI
check: release-check dashboard-check
	cargo fmt --all -- --check
	cargo nextest run --locked
	cargo clippy --bin lg-hue-sync -- -D warnings
	git diff --check

## default TV build; never silently fall back to another toolchain
build: build-native

## explicit Docker fallback matching webOS 6.x glibc 2.28
build-docker: cross-image
	@printf "$(CYAN)[*] Cross-compiling for $(TARGET) in Debian Buster container...$(RESET)\n"
	docker run --rm \
	  -v "$$PWD":/app -w /app \
	  -v $(CARGO_CACHE):/cargo-cache \
	  -v $(TARGET_CACHE):/target-cache \
	  $(CROSS_IMAGE)
	@printf "$(GREEN)[+] Build complete: $(BINARY) ($$(du -h $(BINARY) | cut -f1))$(RESET)\n"

## prepare reusable host-native cross tools from an existing SDK and cached builder
native-setup:
	WEBOS_NATIVE_HOME="$(WEBOS_NATIVE_HOME)" CROSS_IMAGE="$(CROSS_IMAGE)" bash scripts/setup-native-toolchain.sh

build-native:
	@test -x "$(WEBOS_NATIVE_HOME)/bin/webos-native-build" || { echo "Native tools missing: run make native-setup, or use make build-docker." >&2; exit 2; }
	bash "$(WEBOS_NATIVE_HOME)/bin/webos-native-build" "$(CURDIR)/Cargo.toml" lg-hue-sync "$(CURDIR)/$(BINARY)"

package-app:
	uv run scripts/package_ipk.py

require-ares-device:
	@test -n "$(ARES_DEVICE)" || { echo "ARES_DEVICE is required; select a registered device explicitly." >&2; exit 2; }

install-app: require-ares-device
	IPK_PATH=$$(uv run scripts/package_ipk.py --print-path); ares-rs-install --device "$(ARES_DEVICE)" "$$IPK_PATH"

launch-app: require-ares-device
	ares-rs-launch --device "$(ARES_DEVICE)" "$(APP_ID)"

list-apps: require-ares-device
	ares-rs-install --device "$(ARES_DEVICE)" --list

## build binary locally on host machine
build-local:
	@printf "$(CYAN)[*] Building host binary with cargo...$(RESET)\n"
	cargo build --release
	@printf "$(GREEN)[+] Host build complete: target/release/lg-hue-sync$(RESET)\n"

## run local unit tests
test:
	@printf "$(CYAN)[*] Running test suite...$(RESET)\n"
	cargo nextest run --locked

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
	IPK_PATH=$$(uv run scripts/package_ipk.py --print-path); scp $(SCP_OPTS) "$$IPK_PATH" root@$(TV_IP):/tmp/org.webosbrew.lg-hue-sync.ipk
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
	ssh -t $(SSH_OPTS) root@$(TV_IP) "$(REMOTE_DIR)/lg-hue-sync pair --output $(REMOTE_DIR)/config.json && systemctl restart lg-hue-sync"

## clean cargo target directory
clean:
	cargo clean

cross-clean:
	@docker image ls --format '{{.Repository}}:{{.Tag}}' --filter 'reference=lg-hue-sync-cross:rust-*' | while IFS= read -r image; do docker image rm "$$image"; done
	-docker volume rm $(CARGO_CACHE) $(TARGET_CACHE)

# Static frontend assets stay embedded: the TV does not need Bun or Node.
.PHONY: dashboard-build dashboard-check dashboard-preview
dashboard-build:
	bun run --cwd frontend build
	cp frontend/dist/index.html src/web/ui.html

dashboard-check:
	bun run --cwd frontend check
	bun run --cwd frontend test
	$(MAKE) dashboard-build

dashboard-preview:
	bun run --cwd frontend dev
