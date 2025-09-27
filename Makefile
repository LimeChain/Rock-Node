# Rock Node Makefile
# Common development tasks and commands

.PHONY: help install test fmt lint check build clean pre-commit setup-hooks

# Default target
help: ## Show this help message
	@echo "Rock Node Development Commands"
	@echo "=============================="
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'

# Setup and Installation
install: ## Install all dependencies and setup development environment
	@echo "📦 Installing Rust components..."
	rustup component add rustfmt clippy
	@echo "🔧 Setting up pre-commit hooks..."
	./.pre-commit-install.sh

setup-hooks: ## Setup pre-commit hooks only
	./.pre-commit-install.sh

# Code Quality
fmt: ## Format all Rust code
	cargo fmt --all

lint: ## Run clippy linter
	cargo clippy --all-targets --all-features --workspace -- -W clippy::pedantic -A clippy::doc_overindented_list_items -A clippy::doc_lazy_continuation -A clippy::large_enum_variant -A unused-imports -A dead-code -A clippy::derivable_impls -A clippy::clone_on_copy -A clippy::needless_borrows_for_generic_args -A clippy::empty_line_after_doc_comments -A clippy::assertions_on_constants

check: ## Check if code compiles
	cargo check --all-targets --all-features --workspace

audit: ## Run security audit
	cargo audit

# Testing
test: ## Run unit tests (excluding e2e)
	cargo test --workspace --exclude e2e

test-all: ## Run all tests including e2e
	cargo test --workspace

test-core: ## Run only core module tests
	cargo test -p rock-node-core

test-coverage: ## Generate test coverage report (requires cargo-llvm-cov)
	cargo llvm-cov --html --open

# Building
build: ## Build the project
	cargo build

build-release: ## Build optimized release version
	cargo build --release

# Cleaning
clean: ## Clean build artifacts
	cargo clean
	rm -rf target/

# Pre-commit
pre-commit: ## Run pre-commit hooks on all files
	pre-commit run --all-files

pre-commit-update: ## Update pre-commit hooks
	pre-commit autoupdate

# Development workflow
dev-check: fmt lint check test ## Run full development check (format, lint, compile, test)
	@echo "✅ All development checks passed!"

ci-check: check lint test ## Run CI-like checks
	@echo "✅ CI checks passed!"

# Quick commands
quick-test: ## Run quick tests for core modules only
	cargo test -p rock-node-core --lib

quick-check: ## Quick compile check
	cargo check --workspace

# Documentation
docs: ## Generate and open documentation
	cargo doc --open --workspace

# Docker
docker-build: ## Build Docker image
	docker build -t rock-node .

docker-test: ## Run tests in Docker
	docker run --rm rock-node cargo test --workspace --exclude e2e

# Utility commands
deps: ## Show dependency tree
	cargo tree

outdated: ## Check for outdated dependencies (requires cargo-outdated)
	cargo outdated

update: ## Update dependencies
	cargo update

# Git hooks
install-hooks: setup-hooks ## Alias for setup-hooks

# Help for new developers
new-dev: install dev-check ## Complete setup for new developers
	@echo ""
	@echo "🎉 Setup complete! You're ready to contribute to Rock Node!"
	@echo ""
	@echo "Next steps:"
	@echo "  • Make your changes"
	@echo "  • Run 'make dev-check' before committing"
	@echo "  • Commit your changes (hooks will run automatically)"
	@echo ""

# Show status
status: ## Show project status
	@echo "Rock Node Project Status"
	@echo "======================="
	@echo "📊 Code Statistics:"
	@find . -name "*.rs" -not -path "./target/*" | wc -l | awk '{print "  Rust files: " $$1}'
	@echo ""
	@echo "🧪 Test Status:"
	@cargo test --workspace --exclude e2e -- --list | grep -c ": test" | awk '{print "  Unit tests: " $$1}' || echo "  Unit tests: 0"
	@echo ""
	@echo "📦 Dependencies:"
	@cargo tree --depth 1 | head -5
	@echo "  ..."
