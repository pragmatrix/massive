build-markdown:
	cd examples/markdown && trunk build --example markdown

dist-out := "/tmp/massive-shell-dist"

build-markdown-release:
	rm -f shell/dist/*
	cd examples/markdown && trunk build --example markdown --release
	mkdir -p {{dist-out}}
	rm -f {{dist-out}}/*
	cp examples/markdown/dist/markdown-*.js {{dist-out}}/massive-markdown.js
	cp examples/markdown/dist/markdown-*_bg.wasm {{dist-out}}/massive-markdown_bg.wasm
	sed -i '' 's/markdown_bg.wasm/massive-markdown_bg.wasm/g' {{dist-out}}/massive-markdown.js

build-code-viewer-release:
	rm -f shell/dist/*
	cd shell && trunk build --example code_viewer --release
	mkdir -p {{dist-out}}
	rm -f {{dist-out}}/*
	cp shell/dist/massive-shell-*.js {{dist-out}}/massive-code.js
	cp shell/dist/massive-shell-*_bg.wasm {{dist-out}}/massive-code_bg.wasm
	sed -i '' 's/massive-shell_bg.wasm/massive-code_bg.wasm/g' {{dist-out}}/massive-code.js

serve-markdown:
	cd examples/markdown && trunk serve --example markdown --port 8888 --no-minification

serve-markdown-release:
	cd examples/markdown && trunk serve --example markdown --port 8888 --no-minification --release

wasm-features:
	cd examples/markdown && cargo tree -f '{p} {f}' --target wasm32-unknown-unknown	

flame:
	cat tracing.folded | inferno-flamegraph > /tmp/massive-trace.svg
	open /tmp/massive-trace.svg
