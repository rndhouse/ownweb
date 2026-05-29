CARGO_MANIFEST := weblayer/Cargo.toml
EXTENSION_DIR := browser-extension
MKDOCS ?= mkdocs

SHARED_JS := \
	$(EXTENSION_DIR)/shared/background.js \
	$(EXTENSION_DIR)/shared/contentScript.js \
	$(EXTENSION_DIR)/shared/options.js \
	$(EXTENSION_DIR)/shared/siteAdapters.js

.PHONY: all build check clean daemon docs extension extension-chrome extension-firefox firefox-lint help test

help:
	@echo "make all               Build the Rust binary, docs site, and browser extensions"
	@echo "make build             Build the WebLayer Rust binary"
	@echo "make check             Run Rust, extension, manifest, and docs checks"
	@echo "make daemon            Run the WebLayer daemon"
	@echo "make test              Run Rust tests"
	@echo "make docs              Build the MkDocs site"
	@echo "make extension         Build Chrome and Firefox extension directories"
	@echo "make extension-chrome  Build the Chrome extension directory"
	@echo "make extension-firefox Build the Firefox extension directory"
	@echo "make firefox-lint      Lint the Firefox extension with web-ext"
	@echo "make clean             Remove generated build output"

all: build docs extension

build:
	cargo build --manifest-path $(CARGO_MANIFEST)

check:
	cargo check --manifest-path $(CARGO_MANIFEST)
	for file in $(SHARED_JS); do node --check "$$file"; done
	node -e "for (const file of ['$(EXTENSION_DIR)/chrome/manifest.json','$(EXTENSION_DIR)/firefox/manifest.json']) { JSON.parse(require('fs').readFileSync(file, 'utf8')); console.log(file + ' ok'); }"
	$(MKDOCS) build --strict

daemon:
	cargo run --manifest-path $(CARGO_MANIFEST) -- daemon

test:
	cargo test --manifest-path $(CARGO_MANIFEST)

docs:
	$(MKDOCS) build --strict

extension:
	$(MAKE) -C $(EXTENSION_DIR) all

extension-chrome:
	$(MAKE) -C $(EXTENSION_DIR) chrome

extension-firefox:
	$(MAKE) -C $(EXTENSION_DIR) firefox

firefox-lint: extension-firefox
	npx --yes web-ext lint --source-dir $(EXTENSION_DIR)/dist/firefox --self-hosted

clean:
	cargo clean --manifest-path $(CARGO_MANIFEST)
	$(MAKE) -C $(EXTENSION_DIR) clean
	rm -rf site
