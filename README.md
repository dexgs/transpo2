# Transpo (2)
Transpo is yet another internet file-sharing service. It is the successor to a
previous project of mine which shares the same name.

## Features
- End-to-end encryption. Files are encrypted in the uploader's browser before
  they are sent to the server and decrypted in the downloader's browser after
  they are received from the server. (See CRYPTO.md)

- Ephemeral uploads. All files uploaded to Transpo will not be stored forever.
  Transpo requires that uploads be configured with a time limit (and optionally
  a download limit) after which they will expire and be deleted from the
  server.

- Optional server-side processing. In addition to end-to-end encryption,
  Transpo can also perform encryption and decryption on the server. This is
  obviously less secure, but enables you to upload/download files from a
  browser without JavaScript support or from the command line with utilities
  like cURL or wget (see [transpo.sh](transpo.sh) for a helper script).

- Multiple database backends. Transpo supports SQLite, PostgreSQL and
  MySQL/MariaDB.

## Usage
Transpo accepts configuration via environment variables and command line
arguments. The configuration is documented by the list below. Each entry follows
the format: `command-line argument` / `environment variable` `<type>` with a
description in a sub-item

- `-a` / `TRANSPO_MAX_UPLOAD_AGE_MINUTES` `<number>`
  - The maximum amount of time in minutes with which an upload may be configured
    before it expires.

- `-u` / `TRANSPO_MAX_UPLOAD_SIZE_BYTES` `<number>`
  - The maximum size of an upload in bytes.

- `-s` / `TRANSPO_MAX_STORAGE_SIZE_BYTES` `<number>`
  - The maximum total size of all uploads currently stored in bytes.

- `-p` / `TRANSPO_PORT` `<number>`
  - The port to which Transpo will bind.

- `-c` / `TRANSPO_COMPRESSION_LEVEL` `<number from 0 to 9 (inclusive)>`
  - The gzip compression level Transpo will use when creating Zip archives on
    the server. (0 disabled compression)

- `-q` / `TRANSPO_QUOTA_BYTES` `<number>`
  - The maximum number of bytes allowed to be uploaded by a single IP address
    within the given quota time period. (0 disables quotas)

- `-i` / `TRANSPO_QUOTA_INTEVAL_MINUTES` `<number>`
  - The interval after which upload quotas will be cleared in minutes.

- `-t` / `TRANSPO_READ_TIMEOUT_MILLISECONDS` `<number>`
  - Timeout in milliseconds before which a client must fill a read buffer/send a
    WebSocket message in order to keep the connection open. This is used to let
    the server close idle connections.

- `-d` / `TRANSPO_STORAGE_DIRECTORY` `<path>`
  - The path to the directory in which Transpo will store uploads.

- `-D` / `TRANSPO_DATABASE_URL` `<path (for SQLite) or URL (for MySQL and PgSQL)>`
  - The connection string for the database Transpo will use

- `-n` / `TRANSPO_APP_NAME` `<string>`
  - Name shown throughout the web interface.

The Transpo executable itself will print this information and exit if it is
called with the `-h` or `--help` command line arguments.

Transpo will print its current configuration to the standard output on startup
unless it is started with `-Q`.

## Compiling
Transpo can be compiled with the following cargo features: 
`sqlite`, `mysql`, and `postgres`. Each feature enables support for its
respective database. Only `sqlite` is enabled by default.

Database support depends on client libraries being available on the system.
- `sqlite` depends on `libsqlite3`
- `postgres` depends on `libpq`
- `mysql` depends on `libmysqlclient`

Additionally, a C toolchain such as `gcc` must be available on the system in
order to link Transpo against the above libraries.

## Proxying
Transpo's web interface must be reached over HTTPS as many of the JavaScript
features on which it depends are only available from a secure context.

Here is a simple NGINX configuration example:
(replace BACKEND-HOST with the host/port at which Transpo is reachable)
```nginx
real_ip_header X-Real-IP;
proxy_http_version 1.1;

location / {
  proxy_pass http://BACKEND-HOST;
}

location /upload {
  client_max_body_size 5G;
  proxy_set_header Upgrade $http_upgrade;
  proxy_set_header Connection "upgrade";

  proxy_pass http://BACKEND-HOST/upload;
}
```

## Docker
Copy `docker-compose.default.yml` to `docker-compose.yml` to make configuration
changes that will not be overwritten by an update.

## Dependencies
The front-end has a single JavaScript dependency:
[client-zip](https://github.com/Touffy/client-zip). Its source & license are
located at ``www/js/transpo/client-zip``.

The back-end's dependencies are declared in ``Cargo.toml``
