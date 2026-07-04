dev:
    cargo watch -x run

install-esquema:
    cargo install esquema-cli --locked --git https://github.com/fatfingers23/esquema.git

gen:
    find ./crates/lexicons/src -type f ! -name .gitkeep -delete
    esquema-cli generate local --lexdir ./lexicons/ --outdir ./crates/lexicons/src
    cargo fmt --package lexicons
