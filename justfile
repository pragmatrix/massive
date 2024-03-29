build-markdown:
	cd shell && trunk build --example markdown

serve-markdown:
	cd shell && trunk serve --example markdown --port 8888 --no-minification

serve-markdown-release:
	cd shell && trunk serve --example markdown --port 8888 --no-minification --release

wasm-features:
	cd shell && cargo tree -f '{p} {f}' --target wasm32-unknown-unknown	

