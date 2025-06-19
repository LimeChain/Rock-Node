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

### Test Coverage

We aim for high test coverage. You can check coverage with:

```bash
# Install cargo-tarpaulin
cargo install cargo-tarpaulin

# Run coverage analysis
cargo tarpaulin --out Html
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

## Questions?

If you have questions about contributing, please:

1. Check this document first
2. Look at existing issues and discussions
3. Create a new issue with the "question" label

Thank you for contributing to Rock Node! 🚀 