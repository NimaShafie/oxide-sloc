.PHONY: help dev check fmt lint test build serve analyze docker-build docker-run clean

help:
	@echo ""
	@echo "  oxide-sloc — available targets"
	@echo ""
	@echo "  Development"
	@echo "    make check          fmt + lint + test (no serve)"
	@echo "    make dev            fmt + lint + test + serve"
	@echo "    make fmt            cargo fmt --all"
	@echo "    make lint           cargo clippy -D warnings"
	@echo "    make test           cargo test --workspace"
	@echo "    make serve          start web UI on http://localhost:3000"
	@echo "    make analyze DIR=.  analyze a directory from the CLI"
	@echo ""
	@echo "  Build"
	@echo "    make build          release binary → target/release/oxidesloc"
	@echo "    make clean          cargo clean"
	@echo ""
	@echo "  Docker"
	@echo "    make docker-build   build Docker image"
	@echo "    make docker-run     run web UI in Docker on port 3000"
	@echo ""

# Run the full dev cycle: format, lint, test, then serve
dev: check serve

# All CI gates — run this before pushing
check: fmt lint test

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --release -p oxidesloc

serve:
	cargo run -p oxidesloc -- serve

# Usage: make analyze DIR=./my-repo
analyze:
	cargo run -p oxidesloc -- analyze $(DIR) --plain

docker-build:
	docker build -t oxide-sloc .

docker-run:
	docker run --rm -p 3000:3000 oxide-sloc

clean:
	cargo clean
