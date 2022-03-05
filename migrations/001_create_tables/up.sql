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
