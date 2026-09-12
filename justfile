set windows-shell := ["powershell.exe", "-c"]

# Build everything in debug mode.
build:
    cargo build --workspace

# Build optimized release binaries (RedStrap.exe + RedStrap-Settings.exe).
release:
    cargo build --workspace --release

# Run the test suite.
test:
    cargo test --workspace --all-features

# Lint with Clippy (deny warnings, as CI does).
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Check formatting.
fmt-check:
    cargo fmt --all -- --check

# Apply formatting.
fmt:
    cargo fmt --all

# Full CI-equivalent check: format, lint, build, test.
check: fmt-check clippy build test

# Remove build output.
clean:
    cargo clean
