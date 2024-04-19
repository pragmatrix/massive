build-markdown:
	cd shell && trunk build --example markdown

dist-out := "/tmp/massive-shell-dist"

build-markdown-release:
	rm shell/dist/*
	cd shell && trunk build --example markdown --release
	mkdir -p {{dist-out}}
	rm -f {{dist-out}}/*
	cp shell/dist/massive-shell-*.js {{dist-out}}/massive-shell.js
	cp shell/dist/massive-shell-*_bg.wasm {{dist-out}}/massive-shell_bg.wasm


serve-markdown:
	cd shell && trunk serve --example markdown --port 8888 --no-minification

serve-markdown-release:
	cd shell && trunk serve --example markdown --port 8888 --no-minification --release

wasm-features:
	cd shell && cargo tree -f '{p} {f}' --target wasm32-unknown-unknown	

flame:
	cat tracing.folded | inferno-flamegraph > /tmp/massive-trace.svg
	open /tmp/massive-trace.svg
