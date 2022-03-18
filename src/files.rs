use std::io::{Result, Error, ErrorKind, BufWriter, Write, Read, BufReader};
use std::path::PathBuf;
use std::fs::{File, OpenOptions};
use std::str;
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{AeadInPlace, Aead, NewAead};
use crate::b64;
use crate::random_bytes::*;
use crate::constants::*;
use chrono::*;
use std::cmp;
use streaming_zip::*;

const MAX_CHUNK_SIZE: usize = FORM_READ_BUFFER_SIZE + 16;


// Writers

// Write to a single file. `start_new_file` can only be called once, calling it
// multiple times returns an error
pub struct FileWriter {
    writer: BufWriter<File>,
    max_upload_size: usize,
    bytes_written: usize,
}

impl FileWriter {
    pub fn new(path: &PathBuf, max_upload_size: usize) -> Result<Self>
    {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;

        let new = Self {
            writer: BufWriter::new(file),
            max_upload_size,
            bytes_written: 0
        };

        Ok(new)
    }
}

impl Write for FileWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.bytes_written += bytes.len();
        if self.bytes_written > self.max_upload_size {
            return Err(other_error());
        }

        self.writer.write_all(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> Result<()> {
        self.writer.flush()
    }
}


// Wrap a FileWriter such that the data written is encrypted with the given key.
// Also encrypts the file name and mime type.
//
// The encrypted file is written as follows:
// - It is divided into segments of varying lengths
//   (but no longer than MAX_CHUNK_SIZE)
// - Each segment is prefixed by a 16-bit unsigned integer in big-endian byte
//   order which stores the length of the segment
// - The file ends with two zero bytes not belonging to any segment.
//
pub struct EncryptedFileWriter {
    writer: FileWriter,
    cipher: Aes256Gcm,
    buffer: Vec<u8>
}

fn encrypt_string(cipher: &Aes256Gcm, string: &str) -> Result<Vec<u8>> {
    match cipher.encrypt(Nonce::from_slice(&[0; 12]), string.as_bytes()) {
        Ok(ciphertext) => Ok(ciphertext),
        Err(_) => Err(other_error())
    }
}

impl EncryptedFileWriter {
    // Return the writer + the b64 encoded key, encrypted file name and encrypted mime type
    pub fn new(path: &PathBuf, max_upload_size: usize, name: &str, mime: &str) -> Result<(Self, Vec<u8>, Vec<u8>, Vec<u8>)>
    {
        let mut key_slice = [0; 32];
        random_bytes(&mut key_slice);
        let encoded_key = b64::base64_encode(&key_slice);
        let key = Key::from_slice(&key_slice);
        let cipher = Aes256Gcm::new(key);
        let writer = FileWriter::new(path, max_upload_size)?;

        let name_cipher = b64::base64_encode(&encrypt_string(&cipher, name)?);
        let mime_cipher = b64::base64_encode(&encrypt_string(&cipher, mime)?);

        let new = Self {
            writer: writer,
            cipher: cipher,
            buffer: Vec::with_capacity(FORM_READ_BUFFER_SIZE * 2)
        };

        Ok((new, encoded_key, name_cipher, mime_cipher))
    }

    pub fn finish(&mut self) -> Result<()> {
        // Make sure the file is terminated by two zero bytes
        self.writer.write(&0u16.to_be_bytes())?;
        Ok(())
    }
}

impl Write for EncryptedFileWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        } else if bytes.len() > MAX_CHUNK_SIZE {
            return Err(other_error());
        }

        if self.buffer.capacity() < bytes.len() * 2 {
            self.buffer.reserve(bytes.len() * 2 - self.buffer.len());
        }

        self.buffer.clear();
        self.buffer.extend_from_slice(bytes);

        match self.cipher.encrypt_in_place(Nonce::from_slice(&[0; 12]), b"", &mut self.buffer) {
            Ok(()) => {
                if self.buffer.len() <= MAX_CHUNK_SIZE {
                    let size_prefix = (self.buffer.len() as u16).to_be_bytes();
                    self.writer.write(&size_prefix)?;
                    self.writer.write(&self.buffer)?;
                    Ok(bytes.len())
                } else {
                    Err(other_error())
                }
            },
            Err(_) => Err(other_error())
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.writer.flush()
    }
}


// Wrap an EncryptedFileWriter such that multiple files can be written into a
// single archive. 
pub struct EncryptedZipWriter {
    writer: Archive<EncryptedFileWriter>,
    compression: CompressionMode,
}

impl EncryptedZipWriter {
    // Return the writer + the b64 encoded key, encrypted file name and encrypted mime type
    pub fn new(path: &PathBuf, max_upload_size: usize, level: u8) -> Result<(Self, Vec<u8>, Vec<u8>, Vec<u8>)> {
        let (inner_writer, key, name, mime) = EncryptedFileWriter::new(
            path, max_upload_size, "", "application/zip")?;
        if level > 9 {
            return Err(Error::from(ErrorKind::InvalidInput));
        }

        let compression = if level == 0 {
            CompressionMode::Store
        } else {
            CompressionMode::Deflate(level)
        };

        let new = Self {
            writer: Archive::new(inner_writer),
            compression
        };

        Ok((new, key, name, mime))
    }

    pub fn start_new_file(&mut self, name: &str) -> Result<()> {
        let now = Local::now().naive_local();
        self.writer.start_new_file(name.to_owned().into_bytes(), now, self.compression, true)
    }

    pub fn finish_file(&mut self) -> Result<()> {
        self.writer.finish_file()
    }

    pub fn finish(self) -> Result<()> {
        let mut inner_writer = self.writer.finish()?;
        inner_writer.finish()?;
        Ok(())
    }
}

impl Write for EncryptedZipWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.writer.append_data(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}


// Readers

// Basic wrapper around a buffered reader for a file.
pub struct FileReader {
    reader: BufReader<File>,
    expire_after: NaiveDateTime,
}

impl FileReader {
    pub fn new(path: &PathBuf, expire_after: NaiveDateTime) -> Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let new = Self {
            reader,
            expire_after
        };

        Ok(new)
    }
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let now = Local::now().naive_local();
        if now > self.expire_after {
            Err(Error::new(ErrorKind::Other, "Upload expired during download"))
        } else {
            self.reader.read(buf)
        }
    }
}


// Wrapper around FileReader. Decrypts its contents with the given key. Also
// decrypts the encrypted name and mime type of the file
pub struct EncryptedFileReader {
    reader: FileReader,
    cipher: Aes256Gcm,
    buffer: Vec<u8>,
    read_start: usize,
    read_end: usize
}

fn decrypt_string(cipher: &Aes256Gcm, bytes: &[u8]) -> Result<String> {
    match cipher.decrypt(Nonce::from_slice(&[0; 12]), bytes) {
        Ok(plaintext) => String::from_utf8(plaintext).or(Err(other_error())),
        Err(_) => Err(other_error())
    }
}

impl EncryptedFileReader {
    // Return the reader + the decrypted file name and decrypted mime type
    pub fn new(
        path: &PathBuf, expire_after: NaiveDateTime, key: &[u8],
        name_cipher: &[u8], mime_cipher: &[u8]) -> Result<(Self, String, String)>
    {
        let key_slice = b64::base64_decode(key).ok_or(other_error())?;
        let key = Key::from_slice(&key_slice);
        let cipher = Aes256Gcm::new(key);

        let name = decrypt_string(&cipher, &b64::base64_decode(name_cipher).ok_or(other_error())?)?;
        let mime = decrypt_string(&cipher, &b64::base64_decode(mime_cipher).ok_or(other_error())?)?;

        let new = Self {
            reader: FileReader::new(path, expire_after)?,
            cipher: cipher,
            buffer: Vec::with_capacity(FORM_READ_BUFFER_SIZE * 2),
            read_start: 0,
            read_end: 0
        };

        Ok((new, name, mime))
    }
}

impl Read for EncryptedFileReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.read_start == self.read_end {
            // if the buffer has no pending decrypted data

            let mut size_buf = 0u16.to_be_bytes();

            if let Err(e) = self.reader.read_exact(&mut size_buf) {
                if e.kind() == ErrorKind::UnexpectedEof {
                    // Trillium will continue trying to read from us, even after
                    // we reach the end of the file. However, returning an error
                    // in this case will cause Trillium to improperly close the
                    // connection to the client which can break the download.
                    //
                    // It's a bit of a hack, but just returning Ok(0) will make
                    // sure Trillium properly terminates the chunk-encoded body.
                    return Ok(0);
                } else {
                    return Err(e);
                }
            }

            let chunk_size = u16::from_be_bytes(size_buf) as usize;

            if chunk_size == 0 {
                return Ok(0); // EOF
            } else if chunk_size > MAX_CHUNK_SIZE {
                return Err(other_error());
            }

            self.buffer.resize(chunk_size, 0);
            self.reader.read_exact(&mut self.buffer)?;

            match self.cipher.decrypt_in_place(Nonce::from_slice(&[0; 12]), b"", &mut self.buffer) {
                Ok(()) => {
                    let plaintext_len = self.buffer.len();
                    let len = cmp::min(buf.len(), plaintext_len);

                    buf[..len].copy_from_slice(&self.buffer[..len]);
                    self.read_start = len;
                    self.read_end = plaintext_len;

                    Ok(len)
                },
                Err(_) => Err(other_error())
            }
        } else {
            // If there is remaining decrypted data that has yet to be sent
            let len = cmp::min(buf.len(), self.read_end - self.read_start);
            buf[..len].copy_from_slice(&self.buffer[self.read_start..][..len]);
            self.read_start += len;

            Ok(len)
        }
    }
}

fn other_error() -> Error {
    Error::from(ErrorKind::Other)
}

pub fn delete_upload_dir(storage_dir: &PathBuf, id: i64) {
    let id_string = String::from_utf8(b64::i64_to_b64_bytes(id)).unwrap();
    let upload_path = storage_dir.join(id_string);
    if upload_path.exists() {
        if let Err(e) = std::fs::remove_dir_all(upload_path) {
            eprintln!("{}", e);
        }
    }
}

pub fn get_storage_size(storage_dir: &PathBuf) -> Result<usize> {
    let mut storage_size = 0;

    for entry in storage_dir.read_dir()? {
        let upload = entry?.path().join("upload");

        if upload.exists() && upload.is_file() {
            let size = File::open(upload)
                .and_then(|f| f.metadata())
                .map(|m| m.len());

            if let Ok(size) = size {
                storage_size += size as usize;
            }
        }
    }

    Ok(storage_size)
}
