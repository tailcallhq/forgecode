# ForgeCode Justfile
set shell := ["bash", "-cu"]

# Show available commands
default:
    @just --list

# Install dependencies
install:
    npm install

# Run evaluation
_eval:
    npm run eval

# Run bounty tests
test:
    npm run test:bounty

# Run linting (eslint + prettier check)
lint:
    npx eslint . --ext .ts
    npx prettier --check "**/*.ts"

# Auto-format code
fmt:
    npx prettier --write "**/*.ts"

# CI-like run (install + eval + test + lint)
ci: install test lint

# Clean artifacts
clean:
    rm -rf node_modules dist
