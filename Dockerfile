# syntax=docker/dockerfile:1
FROM alpine:latest

ARG TRANSPO_STORAGE_DIRECTORY

ENV TRANSPO_STORAGE_DIRECTORY ${TRANSPO_STORAGE_DIRECTORY:-./transpo_storage}

WORKDIR /transpo
COPY . .

RUN adduser -D transpo
RUN mkdir -p ${TRANSPO_STORAGE_DIRECTORY}
RUN chown -R transpo:transpo ${TRANSPO_STORAGE_DIRECTORY}

RUN apk add cargo gcc sqlite-dev libpq-dev mariadb-connector-c-dev
RUN cargo build --release --all-features

USER transpo
CMD ./target/release/transpo2
