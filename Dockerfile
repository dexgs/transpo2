# syntax=docker/dockerfile:1
FROM alpine:latest

ARG TRANSPO_STORAGE_DIRECTORY
ARG FEATURES

ENV TRANSPO_STORAGE_DIRECTORY ${TRANSPO_STORAGE_DIRECTORY:-./transpo_storage}

WORKDIR /transpo
COPY . .

RUN adduser -D transpo
RUN mkdir -p ${TRANSPO_STORAGE_DIRECTORY}
RUN chown -R transpo:transpo ${TRANSPO_STORAGE_DIRECTORY}

RUN apk add cargo gcc musl-dev sqlite-dev libpq-dev mariadb-connector-c-dev
RUN cargo build --release --features ${FEATURES:-"sqlite,postgres,mysql"}

USER transpo
CMD ./target/release/transpo2
