# syntax=docker/dockerfile:1
FROM alpine:edge AS builder

ARG FEATURES="sqlite,postgres,mysql"

WORKDIR /transpo
COPY . .

RUN apk add cargo gcc musl-dev sqlite-dev libpq-dev mariadb-connector-c-dev
RUN cargo build --release --no-default-features --features $FEATURES
RUN strip target/release/transpo2

RUN mkdir -p pkg
RUN mv target/release/transpo2 pkg
RUN mv templates pkg
RUN mv www pkg
RUN mv migrations pkg
RUN mv pg_migrations pkg


FROM alpine:latest

WORKDIR /transpo

COPY --from=builder /transpo/pkg .

RUN apk add libgcc sqlite-libs libpq mariadb-connector-c
RUN adduser -D transpo
RUN mkdir -p /transpo_storage && chown -R transpo:transpo /transpo_storage

USER transpo
CMD ["./transpo2", "-d", "/transpo_storage", "-Q"]
