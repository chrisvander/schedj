dev:
    cargo watch -x run

demo:
    #!/usr/bin/env bash
    set -euo pipefail
    podman compose up -d postgres
    for attempt in {1..30}; do
        if podman compose exec -T postgres pg_isready -U schedj -d schedj >/dev/null; then
            break
        fi
        if [[ "$attempt" == 30 ]]; then
            echo "PostgreSQL did not become ready within 30 seconds" >&2
            exit 1
        fi
        sleep 1
    done
    exec env DATABASE_URL=postgres://schedj:schedj@localhost:5432/schedj cargo run -p schedj

db-up:
    podman compose up -d postgres

db-down:
    podman compose down

install-esquema:
    cargo install esquema-cli --locked --git https://github.com/fatfingers23/esquema.git

gen:
    find ./crates/lexicons/src -type f ! -name .gitkeep -delete
    esquema-cli generate local --lexdir ./lexicons/ --outdir ./crates/lexicons/src
    cargo fmt --package lexicons
