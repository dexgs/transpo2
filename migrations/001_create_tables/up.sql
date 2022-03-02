-- data about the uploads currently stored
CREATE TABLE IF NOT EXISTS uploads (
    id BIGINT PRIMARY KEY,
    file_name TEXT NOT NULL,
    mime_type VARCHAR(512) NOT NULL,
    password_hash BINARY(96),
    remaining_downloads INT,
    num_accessors INT,
    expire_after TIMESTAMP NOT NULL
);

-- this table only ever holds 1 row to keep track of storage
CREATE TABLE IF NOT EXISTS storage_size (
    id INT PRIMARY KEY,
    num_bytes BIGINT NOT NULL
);
