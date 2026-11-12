build-markdown:
	cd examples/markdown && trunk build --example markdown

dist-out := "/tmp/massive-dist"

build-markdown-release:
	rm -f examples/markdown/dist/*
	cd examples/markdown && trunk build --example markdown --release
	mkdir -p {{dist-out}}
	rm -f {{dist-out}}/*
	cp examples/markdown/dist/markdown-*.js {{dist-out}}/massive-markdown.js
	cp examples/markdown/dist/markdown-*_bg.wasm {{dist-out}}/massive-markdown_bg.wasm
	sed -i '' 's/markdown_bg.wasm/massive-markdown_bg.wasm/g' {{dist-out}}/massive-markdown.js

build-code-viewer-release:
	rm -f examples/code/dist/*
	cd examples/code && trunk build --example code-viewer --release
	mkdir -p {{dist-out}}
	rm -f {{dist-out}}/*
	cp examples/code/dist/code-*.js {{dist-out}}/massive-code.js
	cp examples/code/dist/code-*_bg.wasm {{dist-out}}/massive-code_bg.wasm
	sed -i '' 's/code_bg.wasm/massive-code_bg.wasm/g' {{dist-out}}/massive-code.js

serve-markdown:
	cd examples/markdown && trunk serve --example markdown --port 8888 --open

serve-markdown-release:
	cd examples/markdown && trunk serve --example markdown --port 8888 --release --open

serve-code-viewer-release:
	cd examples/code && trunk serve --example code-viewer --port 8888 --release --open

wasm-features:
	cd examples/markdown && cargo tree -f '{p} {f}' --target wasm32-unknown-unknown	

flame:
	cat tracing.folded | inferno-flamegraph > /tmp/massive-trace.svg
	open /tmp/massive-trace.svg

run:
	cd .. && cargo run 

# The lint and format checks of .github/workflows/qa.yaml, to run before pushing.
qa: lints-opt-in
	cargo fmt -- --check
	cargo clippy --all-targets -- -D warnings
	cargo test --release

# Every workspace member of this repository opts into the workspace lints, because clippy applies
# them only to crates that do, and a crate without the opt-in passes `qa` silently. Members inside
# a submodule, such as the vendored inlyne, are foreign code and skipped.
lints-opt-in:
	#!/bin/sh
	root=$(git rev-parse --show-toplevel)
	missing=$(cargo metadata --no-deps --format-version 1 2>/dev/null | jq -r '.packages[].manifest_path' |
		while read -r manifest; do
			[ "$(git -C "$(dirname "$manifest")" rev-parse --show-toplevel)" = "$root" ] || continue
			grep -q '^\[lints\]' "$manifest" || echo "$manifest"
		done)
	if [ -n "$missing" ]; then
		echo "These crates lack '[lints] workspace = true':"
		echo "$missing"
		exit 1
	fi
