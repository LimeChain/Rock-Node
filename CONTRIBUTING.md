# Contributing to Rock Node

Thank you for your interest in contributing to Rock Node! This document provides guidelines and information for contributors.

## Table of Contents

- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Code Style](#code-style)
- [Testing](#testing)
- [Submitting Changes](#submitting-changes)
- [Issue Reporting](#issue-reporting)
- [Feature Requests](#feature-requests)
- [Documentation](#documentation)
- [Community](#community)

## Getting Started

Before you begin contributing, please:

1. Read this contributing guide
2. Familiarize yourself with the [Code of Conduct](CODE_OF_CONDUCT.md)
3. Check existing issues and pull requests to avoid duplication
4. Join our community discussions

## Development Setup

### Prerequisites

- Rust 1.75.0 or later
- Cargo (Rust's package manager)
- Git
- Docker (optional, for containerized development)

### Local Development

1. Fork the repository
2. Clone your fork:
   ```bash
   git clone https://github.com/yourusername/rock-node.git
   cd rock-node
   ```

3. Add the upstream remote:
   ```bash
   git remote add upstream https://github.com/original-owner/rock-node.git
   ```

4. Build the project:
   ```bash
   cargo build
   ```

5. Run tests:
   ```bash
   cargo test
   ```

## Code Style

### Rust Code Style

We follow Rust community standards and use `rustfmt` for code formatting:

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt --check
```

### Clippy

We use Clippy for additional linting:

```bash
# Run Clippy
cargo clippy

# Run Clippy with all warnings
cargo clippy -- -W clippy::all
```

### Code Organization

- Follow Rust naming conventions
- Use meaningful variable and function names
- Add comprehensive documentation for public APIs
- Keep functions focused and concise
- Use appropriate error handling with `Result` and `Option`

### Documentation

- Document all public APIs with doc comments
- Include examples in documentation where appropriate
- Keep README files up to date
- Update relevant documentation when adding new features

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run integration tests
cargo test --test integration_test_name
```

### Test Guidelines

- Write unit tests for new functionality
- Include integration tests for complex features
- Ensure tests are deterministic and don't depend on external state
- Use meaningful test names that describe the behavior being tested
- Mock external dependencies appropriately

### Testing with Metrics

When writing tests that use Prometheus metrics:

- **Always use isolated registries** to prevent cardinality conflicts
- Use `rock_node_core::test_utils::create_isolated_metrics()` for test contexts
- Avoid using `MetricsRegistry::new()` directly in tests
- Follow the established patterns in existing plugins

```rust
use rock_node_core::test_utils::create_isolated_metrics;

#[test]
fn my_metrics_test() {
    let metrics = create_isolated_metrics();
    // Test implementation using isolated metrics...
}
```

See [Registry Isolation Guide](docs/registry-isolation.md) for detailed information.

### Test Coverage

We aim for high test coverage. You can check coverage with:

```bash
# Install cargo-tarpaulin
cargo install cargo-tarpaulin

# Run coverage analysis
cargo tarpaulin --out Html
```

### Development Workflow

#### Recommended Daily Workflow

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

#### Available Make Commands

```bash
make help              # Show all available commands
make install           # Setup development environment
make dev-check         # Run full development check
make quick-check       # Quick compilation check
make clean             # Clean build artifacts
make docs              # Generate documentation
```

## Code Quality Standards

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

## CI/CD Integration

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

## Troubleshooting

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

## Submitting Changes

### Pull Request Process

1. **Create a feature branch** from the main branch:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** following the code style guidelines

3. **Write tests** for new functionality

4. **Update documentation** as needed

5. **Commit your changes** with clear, descriptive commit messages:
   ```bash
   git commit -m "feat: add new block verification feature"
   ```

6. **Push to your fork**:
   ```bash
   git push origin feature/your-feature-name
   ```

7. **Create a Pull Request** with:
   - Clear description of changes
   - Reference to related issues
   - Screenshots (if UI changes)
   - Test results

### Commit Message Format

We use conventional commit messages:

- `feat:` - New features
- `fix:` - Bug fixes
- `docs:` - Documentation changes
- `style:` - Code style changes (formatting, etc.)
- `refactor:` - Code refactoring
- `test:` - Adding or updating tests
- `chore:` - Maintenance tasks

### Pull Request Guidelines

- Keep PRs focused and reasonably sized
- Include tests for new functionality
- Update documentation as needed
- Respond to review comments promptly
- Ensure CI checks pass

## Issue Reporting

### Bug Reports

When reporting bugs, please include:

- Clear description of the issue
- Steps to reproduce
- Expected vs actual behavior
- Environment details (OS, Rust version, etc.)
- Error messages or logs
- Minimal reproduction case if possible

### Issue Templates

Use the appropriate issue template when creating new issues:

- Bug report template for bugs
- Feature request template for new features
- Documentation issue template for documentation problems

## Feature Requests

When requesting features:

- Describe the use case clearly
- Explain the benefits
- Consider implementation complexity
- Check if similar features already exist
- Provide examples if possible

## Documentation

### Contributing to Documentation

- Keep documentation up to date with code changes
- Use clear, concise language
- Include code examples where helpful
- Follow the existing documentation style
- Update README files when adding new features

### Documentation Structure

- `README.md` - Project overview and quick start
- `docs/` - Detailed documentation
- Inline code documentation
- API documentation

## Community

### Getting Help

- Check existing issues and discussions
- Join community channels (if available)
- Ask questions in GitHub Discussions
- Review documentation

### Code Review

- Be respectful and constructive in reviews
- Focus on the code, not the person
- Provide specific, actionable feedback
- Ask questions when something is unclear
- Suggest improvements constructively

### Recognition

Contributors will be recognized in:

- GitHub contributors list
- Release notes
- Project documentation (if appropriate)

## License

By contributing to Rock Node, you agree that your contributions will be licensed under the same license as the project (Apache License 2.0).

## Additional Resources

- [Rust Book](https://doc.rust-lang.org/book/)
- [Clippy Lint Documentation](https://rust-lang.github.io/rust-clippy/master/)
- [rustfmt Configuration](https://rust-lang.github.io/rustfmt/)
- [Pre-commit Documentation](https://pre-commit.com/)

## Questions?

If you have questions about contributing, please:

1. Check this document first
2. Look at existing issues and discussions
3. Create a new issue with the "question" label

Thank you for contributing to Rock Node! 🚀 