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

# The checks of .github/workflows/qa.yaml, plus the lint opt-in check CI lacks, to run before
# pushing.
qa: lints-opt-in
	cargo fmt -- --check
	cargo clippy --all-targets -- -D warnings
	cargo test --release

# Every workspace member of this repository opts into the workspace lints, because clippy applies
# them only to crates that do, and a crate without the opt-in passes `qa` silently. Members inside
# a submodule, such as the vendored inlyne, are foreign code and skipped.
#
# Each step exits on failure, so a missing tool or a failing `cargo metadata` fails the check
# instead of letting it pass without looking at any crate.
lints-opt-in:
	#!/bin/sh
	set -eu
	command -v jq >/dev/null || { echo "lints-opt-in needs jq"; exit 1; }
	root=$(git rev-parse --show-toplevel)
	metadata=$(cargo metadata --no-deps --format-version 1)
	manifests=$(printf '%s' "$metadata" | jq -r '.packages[].manifest_path')
	missing=$(printf '%s\n' "$manifests" | while read -r manifest; do
		[ "$(git -C "$(dirname "$manifest")" rev-parse --show-toplevel)" = "$root" ] || continue
		awk '
			/^\[/ { in_lints = ($0 ~ /^\[lints\][ \t]*$/) }
			in_lints && /^[ \t]*workspace[ \t]*=[ \t]*true/ { found = 1 }
			/^[ \t]*lints\.workspace[ \t]*=[ \t]*true/ { found = 1 }
			END { exit !found }
		' "$manifest" || echo "$manifest"
	done)
	if [ -n "$missing" ]; then
		echo "These crates do not opt into the workspace lints with '[lints] workspace = true':"
		echo "$missing"
		exit 1
	fi
