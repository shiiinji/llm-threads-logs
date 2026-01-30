-include .env
export

.PHONY: build release install uninstall obsidian-dirs check-env

BIN_DIR ?= $(HOME)/.local/bin

build:
	cargo build

release:
	cargo build --release

check-env:
	@test -n "$(OBSIDIAN_VAULT)" || (echo "OBSIDIAN_VAULT is required" && exit 1)
	@test -n "$(OBSIDIAN_AI_ROOT)" || (echo "OBSIDIAN_AI_ROOT is required" && exit 1)

obsidian-dirs: check-env
	mkdir -p "$(OBSIDIAN_VAULT)/$(OBSIDIAN_AI_ROOT)"

install: release
	mkdir -p "$(BIN_DIR)"
	cp target/release/claude_session_to_obsidian "$(BIN_DIR)/"
	cp target/release/codex_notify_to_obsidian "$(BIN_DIR)/"

uninstall:
	rm -f "$(BIN_DIR)/claude_session_to_obsidian" "$(BIN_DIR)/codex_notify_to_obsidian"
