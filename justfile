run:
    cargo run --profile fast -p helix

run-release:
    cargo run --release -p helix

build:
    cargo build --release -p helix

check:
    cargo check --workspace

dev:
    cargo run -p helix

bundle:
    ./scripts/bundle-mac.sh

release: bundle
    open target/Helix.app
