run:
    cargo run --profile fast -p helix

run-release:
    cargo run --release -p helix

build:
    cargo build --release -p helix

check:
    cargo check --workspace

# Rebuild and relaunch on every change (assets included: icons and fonts are compiled in)
dev:
    cargo watch -c -w src -w assets -w Cargo.toml -x 'run --profile fast -p helix'

bundle:
    ./scripts/bundle-mac.sh

release: bundle
    open target/Helix.app
