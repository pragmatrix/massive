trunk:
	cd shell && trunk build --example markdown

serve:
	cd shell && trunk serve --example markdown --port 8888 --no-minification

trunk-release:
	cd shell && trunk build --release --example markdown

wasm-features:
	cd shell && cargo tree -f '{p} {f}' --target wasm32-unknown-unknown	

