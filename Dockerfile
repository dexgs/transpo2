# syntax=docker/dockerfile:1
FROM alpine:edge AS binary

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


FROM alpine:3 AS base

ARG FEATURES="sqlite,postgres,mysql"

WORKDIR /transpo

COPY --from=binary /transpo/pkg .

RUN apk add --no-cache libgcc `echo $FEATURES \
    | sed 's/,/ /g' \
    | sed 's/sqlite/sqlite-libs/' \
    | sed 's/postgres/libpq/' \
    | sed 's/mysql/mariadb-connector-c/'`
RUN adduser -D transpo
RUN mkdir -p /transpo_storage && chown -R transpo:transpo /transpo_storage


FROM base
USER transpo
CMD ["./transpo2", "-Q", "-d", "/transpo_storage"]
