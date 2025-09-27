#!/bin/bash

# Pre-commit setup script for Rock Node
# This script installs and configures pre-commit hooks

set -euo pipefail

echo "🔧 Setting up pre-commit hooks for Rock Node..."

# Check if Python is available
if ! command -v python3 &> /dev/null && ! command -v python &> /dev/null; then
    echo "❌ Python is required to install pre-commit"
    echo "Please install Python 3.6+ and try again"
    exit 1
fi

# Determine Python command
PYTHON_CMD=""
if command -v python3 &> /dev/null; then
    PYTHON_CMD="python3"
elif command -v python &> /dev/null; then
    PYTHON_CMD="python"
fi

echo "📦 Installing pre-commit..."

# Install pre-commit using pip
$PYTHON_CMD -m pip install --user pre-commit

# Verify installation
if ! command -v pre-commit &> /dev/null; then
    echo "❌ pre-commit installation failed or not in PATH"
    echo "Please ensure ~/.local/bin is in your PATH"
    echo "Add this to your shell profile: export PATH=\"\$HOME/.local/bin:\$PATH\""
    exit 1
fi

echo "✅ pre-commit installed successfully"

# Install the git hooks
echo "🔗 Installing git hooks..."
pre-commit install

# Install hooks for commit messages (optional)
pre-commit install --hook-type commit-msg

echo "🧪 Running pre-commit on all files (this may take a while)..."
# Run on all files to ensure everything is set up correctly
pre-commit run --all-files || {
    echo "⚠️  Some checks failed. This is normal for the first run."
    echo "Pre-commit has been installed and will run on future commits."
}

echo ""
echo "✅ Pre-commit setup complete!"
echo ""
echo "📋 What happens now:"
echo "  • Pre-commit hooks will run automatically on each commit"
echo "  • Code will be formatted with rustfmt"
echo "  • Code will be linted with clippy"
echo "  • Unit tests will run (excluding e2e tests)"
echo "  • Basic file checks will be performed"
echo ""
echo "🛠️  Manual commands:"
echo "  • Run on all files: pre-commit run --all-files"
echo "  • Update hooks: pre-commit autoupdate"
echo "  • Skip hooks (if needed): git commit --no-verify"
echo ""
echo "🚀 Happy coding!"
