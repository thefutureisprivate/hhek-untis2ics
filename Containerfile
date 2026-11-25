FROM docker.io/library/rust:1-alpine@sha256:8d0e82abfdf9a38071e07cabc5afba4ddbe077631a80e4e742a760c38beadc33 AS verify
WORKDIR /app
RUN apk upgrade --no-cache \
    && apk add --no-cache build-base ca-certificates tzdata \
    && rustup component add rustfmt clippy
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo fmt --check \
    && cargo clippy --locked --all-targets -- -D warnings \
    && cargo test --locked

FROM verify AS build
COPY licenses /notices
RUN cargo build --release --locked \
    && mkdir -p /notices/crates /notices/rust \
    && cp /usr/local/rustup/toolchains/*/share/doc/rust/COPYRIGHT-library.html /notices/rust/ \
    && cp -r /usr/local/rustup/toolchains/*/share/doc/rust/licenses /notices/rust/ \
    && cd /usr/local/cargo/registry/src \
    && find . -type f \( -iname '*license*' -o -iname '*licence*' \
        -o -iname '*copying*' -o -iname '*notice*' -o -iname '*copyright*' \
        -o -ipath '*/licenses/*' \) \
        -exec sh -eu -c 'for notice do install -Dm444 "$notice" "/notices/crates/$notice"; done' sh {} + \
    && chmod -R a+rX /notices

FROM scratch
COPY --chmod=0444 LICENSE /LICENSE
COPY --from=build /notices /usr/share/licenses/hhek-untis2ics
LABEL org.opencontainers.image.source="https://github.com/thefutureisprivate/hhek-untis2ics"
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /usr/share/zoneinfo/Europe/Berlin /usr/share/zoneinfo/Europe/Berlin
COPY --from=build /app/target/release/hhek-untis2ics /hhek-untis2ics
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
USER 65532:65532
EXPOSE 8080 9090
ENTRYPOINT ["/hhek-untis2ics"]
