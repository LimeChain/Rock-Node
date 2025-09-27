# Rock Node Development Guide

This guide helps you set up a development environment for Rock Node and explains the code quality tools and processes.

## 🚀 Quick Start for New Developers

```bash
# 1. Clone the repository
git clone <repository-url>
cd Rock-Node

# 2. Run the complete setup
make new-dev
```

This single command will:
- Install Rust components (rustfmt, clippy)
- Set up pre-commit hooks
- Run a full development check
- Verify everything is working

## 🛠️ Development Tools

### Pre-commit Hooks

We use [pre-commit](https://pre-commit.com/) to automatically run code quality checks before each commit:

- **rustfmt**: Automatically formats Rust code
- **clippy**: Lints Rust code for common issues
- **cargo check**: Ensures code compiles
- **cargo test**: Runs unit tests (excluding e2e)
- **Security audit**: Checks for known vulnerabilities
- **General checks**: Trailing whitespace, file endings, etc.

#### Installation

```bash
# Automatic (recommended)
make install

# Manual
./.pre-commit-install.sh

# Or using pip directly
pip install pre-commit
pre-commit install
```

#### Usage

Pre-commit hooks run automatically on `git commit`. You can also run them manually:

```bash
# Run all hooks on staged files
pre-commit run

# Run all hooks on all files
pre-commit run --all-files

# Skip hooks (if absolutely necessary)
git commit --no-verify
```

### Code Formatting

We use `rustfmt` with a custom configuration ([rustfmt.toml](rustfmt.toml)):

```bash
# Format all code
make fmt
# or
cargo fmt --all
```

**Key formatting rules:**
- 100 character line width
- 4 spaces indentation
- Imports grouped by std/external/crate
- Trailing commas in vertical layouts

### Linting

We use `clippy` with custom configuration ([clippy.toml](clippy.toml)):

```bash
# Run linter
make lint
# or
cargo clippy --all-targets --all-features --workspace -- -D warnings
```

**Lint configuration:**
- Cognitive complexity threshold: 30
- Function length limit: 100 lines
- Arguments limit: 7 per function
- All warnings treated as errors in CI

## 🧪 Testing

### Running Tests

```bash
# Unit tests only (fast)
make test

# All tests including e2e (slow)
make test-all

# Core module tests only
make test-core

# Quick test for development
make quick-test
```

### Test Coverage

```bash
# Generate coverage report (requires cargo-llvm-cov)
make test-coverage
```

### Writing Tests

Follow the established patterns in the codebase:

- Unit tests in `#[cfg(test)] mod tests`
- Use `#[tokio::test]` for async tests
- Use isolated test environments (tempfile, isolated metrics)
- Test both success and error scenarios

## 🔧 Development Workflow

### Recommended Daily Workflow

```bash
# 1. Start your work
git checkout -b feature/my-feature

# 2. Make your changes
# ... edit code ...

# 3. Run development checks (before committing)
make dev-check

# 4. Commit your changes
git add .
git commit -m "feat: add my feature"
# Pre-commit hooks run automatically

# 5. Push and create PR
git push origin feature/my-feature
```

### Available Make Commands

```bash
make help              # Show all available commands
make install           # Setup development environment
make dev-check         # Run full development check
make quick-check       # Quick compilation check
make clean             # Clean build artifacts
make docs              # Generate documentation
```

## 📊 Code Quality Standards

### Compilation
- All code must compile without warnings
- Use `#[allow(...)]` sparingly and with justification

### Formatting
- Code must be formatted with `rustfmt`
- No manual formatting overrides

### Linting
- All clippy warnings must be addressed
- Prefer fixing the issue over allowing the lint
- Document any necessary `#[allow(...)]` with comments

### Testing
- New code should include appropriate tests
- Tests should cover both success and error paths
- Use descriptive test names: `test_function_scenario_outcome`

### Documentation
- Public APIs should have doc comments
- Use `///` for public documentation
- Include examples in doc comments where helpful

## 🔄 CI/CD Integration

### GitHub Actions

The project includes several CI workflows:

- **Code Quality** ([.github/workflows/code-quality.yml](.github/workflows/code-quality.yml))
  - Formatting check
  - Clippy linting
  - Compilation check
  - Security audit
  - Pre-commit validation

- **Unit Tests** ([.github/workflows/unit-tests.yml](.github/workflows/unit-tests.yml))
  - Run unit tests across platforms
  - Generate coverage reports

- **E2E Tests** ([.github/workflows/e2e-tests.yml](.github/workflows/e2e-tests.yml))
  - Integration testing with Docker

### Status Checks

All PRs must pass:
- ✅ Code formatting (`cargo fmt --check`)
- ✅ Linting (`cargo clippy`)
- ✅ Compilation (`cargo check`)
- ✅ Unit tests (`cargo test`)
- ✅ Security audit (`cargo audit`)

## 🐛 Troubleshooting

### Pre-commit Issues

```bash
# Update hooks
pre-commit autoupdate

# Clear cache
pre-commit clean

# Reinstall hooks
pre-commit uninstall
pre-commit install
```

### Formatting Conflicts

```bash
# Check what rustfmt would change
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all
```

### Clippy Issues

```bash
# See detailed clippy output
cargo clippy --all-targets --all-features --workspace

# Fix automatically fixable issues
cargo clippy --fix --all-targets --all-features --workspace
```

## 📚 Additional Resources

- [Rust Book](https://doc.rust-lang.org/book/)
- [Clippy Lint Documentation](https://rust-lang.github.io/rust-clippy/master/)
- [rustfmt Configuration](https://rust-lang.github.io/rustfmt/)
- [Pre-commit Documentation](https://pre-commit.com/)

## 🤝 Contributing

1. Follow the development workflow above
2. Ensure all code quality checks pass
3. Write tests for new functionality
4. Update documentation as needed
5. Create descriptive commit messages
6. Submit a pull request

Happy coding! 🦀