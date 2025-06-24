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
	cd examples/markdown && trunk serve --example markdown --port 8888 --no-minification

serve-markdown-release:
	cd examples/markdown && trunk serve --example markdown --port 8888 --release --open

serve-code-viewer-release:
	cd examples/code && trunk serve --example code-viewer --port 8888 --release --open

wasm-features:
	cd examples/markdown && cargo tree -f '{p} {f}' --target wasm32-unknown-unknown	

flame:
	cat tracing.folded | inferno-flamegraph > /tmp/massive-trace.svg
	open /tmp/massive-trace.svg
