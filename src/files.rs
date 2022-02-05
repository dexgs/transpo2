use std::io::{Result, Error, ErrorKind, BufWriter, Write, Seek, SeekFrom, Read, BufRead, BufReader};
use std::path::PathBuf;
use std::fs::File;
use std::str;
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{AeadInPlace, Aead, NewAead};
use crate::b64;
use crate::random_bytes::*;
use crate::constants::*;
use zip::write::{FileOptions, ZipWriter};
use std::boxed::Box;
use std::cmp;


// Writers


pub trait TranspoFileWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize>;
}


// Write to a single file. `start_new_file` can only be called once, calling it
// multiple times returns an error
pub struct FileWriter {
    writer: BufWriter<File>,
}

impl FileWriter {
    pub fn new(path: &PathBuf) -> Result<Self> {
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(path)?;

        let new = Self {
            writer: BufWriter::new(file),
        };

        Ok(new)
    }
}

impl TranspoFileWriter for FileWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.writer.write(bytes)
    }
}


// Wrap a FileWriter such that the data written is encrypted with the given key.
// Also encrypts the file name and mime type.
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
    pub fn new(path: &PathBuf, name: &str, mime: &str) -> Result<(Self, Vec<u8>, Vec<u8>, Vec<u8>)> {
        let key_slice = random_bytes(32);
        let encoded = b64::base64_encode(&key_slice);
        let key = Key::from_slice(&key_slice);
        let cipher = Aes256Gcm::new(key);
        let mut writer = FileWriter::new(path)?;

        let name_cipher = encrypt_string(&cipher, name)?;
        let mime_cipher = encrypt_string(&cipher, mime)?;

        let new = Self {
            writer: writer,
            cipher: cipher,
            buffer: Vec::with_capacity(FORM_READ_BUFFER_SIZE * 2)
        };

        Ok((new, encoded, name_cipher, mime_cipher))
    }
}

impl TranspoFileWriter for EncryptedFileWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self.buffer.capacity() < bytes.len() * 2 {
            self.buffer.reserve(bytes.len() * 4 - self.buffer.len());
        }

        match self.cipher.encrypt_in_place(Nonce::from_slice(&[0; 12]), bytes, &mut self.buffer) {
            Ok(_) => {
                self.writer.write(&self.buffer)
            },
            Err(_) => Err(other_error())
        }
    }
}

pub enum Writer {
    Basic(FileWriter),
    Encrypted(EncryptedFileWriter),
    None
}

impl Writer {
    pub fn is_none(&self) -> bool {
        match self {
            Writer::None => true,
            _ => false
        }
    }
}


// Readers


// Basic wrapper around a buffered reader for a file.
pub struct FileReader {
    reader: BufReader<File>,
}

impl FileReader {
    pub fn new(path: &PathBuf) -> Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let new = Self {
            reader: reader
        };

        Ok(new)
    }
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.reader.read(buf)
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
    pub fn new(path: &PathBuf, key: &[u8], name_cipher: &[u8], mime_cipher: &[u8]) -> Result<(Self, String, String)> {
        let key_slice = b64::base64_decode(key);
        let key = Key::from_slice(&key_slice);
        let cipher = Aes256Gcm::new(key);

        let name = decrypt_string(&cipher, name_cipher)?;
        let mime = decrypt_string(&cipher, mime_cipher)?;

        let new = Self {
            reader: FileReader::new(path)?,
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
        if self.read_start == self.read_end {
            // if the buffer has no pending decrypted data

            let bytes_read = self.reader.read(buf)?;

            if bytes_read == 0 {
                return Ok(0); // return early
            }

            let buf = &mut buf[..bytes_read];

            if self.buffer.capacity() < bytes_read * 2 {
                self.buffer.reserve(bytes_read * 4 - self.buffer.len());
            }

            match self.cipher.decrypt_in_place(Nonce::from_slice(&[0; 12]), buf, &mut self.buffer) {
                Ok(_) => {
                    buf.copy_from_slice(&self.buffer[..bytes_read]);
                    self.read_start = bytes_read;
                    self.read_end = self.buffer.len();

                    Ok(bytes_read)
                },
                Err(_) => Err(other_error())
            }
        } else {
            // If there is remaining decrypted data that has yet to be sent
            let num_bytes = cmp::min(buf.len(), self.read_end - self.read_start);
            buf.copy_from_slice(&self.buffer[self.read_start..self.read_end]);
            self.read_start += num_bytes;

            Ok(num_bytes)
        }
    }
}

pub enum Reader {
    Basic(FileReader),
    Encrypted(EncryptedFileReader),
    None
}

impl Reader {
    pub fn is_none(&self) -> bool {
        match self {
            Reader::None => true,
            _ => false
        }
    }
}


fn other_error() -> Error {
    Error::from(ErrorKind::Other)
}
