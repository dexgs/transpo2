# syntax=docker/dockerfile:1
FROM alpine:edge AS builder

ARG TRANSPO_STORAGE_DIRECTORY
ARG FEATURES

WORKDIR /transpo
COPY . .

RUN apk add cargo gcc musl-dev sqlite-dev libpq-dev mariadb-connector-c-dev
RUN cargo build --release --no-default-features --features ${FEATURES:-sqlite,postgres,mysql}
RUN strip target/release/transpo2

RUN mkdir -p pkg
RUN mv target/release/transpo2 pkg
RUN mv templates pkg
RUN mv www pkg
RUN mv migrations pkg
RUN mv pg_migrations pkg


FROM alpine:latest

ARG TRANSPO_STORAGE_DIRECTORY
ENV TRANSPO_STORAGE_DIRECTORY ${TRANSPO_STORAGE_DIRECTORY:-/transpo_storage}

WORKDIR /transpo

COPY --from=builder /transpo/pkg .

RUN apk add libgcc sqlite-libs libpq mariadb-connector-c
RUN adduser -D transpo
RUN mkdir -p ${TRANSPO_STORAGE_DIRECTORY} && chown -R transpo:transpo ${TRANSPO_STORAGE_DIRECTORY}

USER transpo
CMD ["./transpo2", "-Q"]
