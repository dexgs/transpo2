use crate::b64;
use crate::random_bytes::*;
use crate::constants::*;

use std::io::{Result, Error, ErrorKind, BufWriter, Write, SeekFrom};
use aes_gcm::aead::{AeadInPlace, Aead, NewAead};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use chrono::{NaiveDateTime, Local};
use std::cmp;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::path::{PathBuf, Path};
use std::pin::{pin, Pin};
use std::str;
use std::task::{Poll, Context};
// use std::time::Duration;
use streaming_zip::*;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, AsyncSeekExt};
use tokio::time::sleep;

const MAX_CHUNK_SIZE: usize = FORM_READ_BUFFER_SIZE + 16;


fn nonce_bytes_from_count(count: &u64) -> [u8; 12] {
    let mut nonce_bytes = [0; 12];
    nonce_bytes[..8].copy_from_slice(&u64::to_le_bytes(*count));
    nonce_bytes
}

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
            return Err(other_error("Maximum upload size exceeded"));
        }

        self.writer.write_all(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> Result<()> {
        self.writer.flush()
    }
}


pub struct AsyncFileWriter {
    writer: tokio::io::BufWriter<tokio::fs::File>,
    max_upload_size: usize,
    bytes_written: usize
}

impl AsyncFileWriter {
    pub async fn new<P>(path: P, max_upload_size: usize) -> Result<Self>
    where P: AsRef<Path> {
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path).await?;

        Ok(Self {
            writer: tokio::io::BufWriter::new(file),
            max_upload_size,
            bytes_written: 0
        })
    }
}

impl AsyncWrite for AsyncFileWriter {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8])
        -> Poll<Result<usize>>
    {
        if self.bytes_written + buf.len() > self.max_upload_size {
            return Poll::Ready(Err(other_error("Maximum upload size exceeded")));
        }

        let f = pin!(&mut self.as_mut().writer).poll_write(cx, buf);
        match f {
            Poll::Ready(Ok(bytes_written)) => {
                self.bytes_written += bytes_written;
                f
            },
            _ => f
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Result<()>>
    {
        pin!(&mut self.as_mut().writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Result<()>>
    {
        pin!(&mut self.as_mut().writer).poll_shutdown(cx)
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
    buffer: Vec<u8>,
    count: u64
}

fn encrypt_string(cipher: &Aes256Gcm, string: &str, count: &mut u64) -> Result<Vec<u8>> {
    let nonce_bytes = nonce_bytes_from_count(count);
    *count += 1;

    match cipher.encrypt(Nonce::from_slice(&nonce_bytes), string.as_bytes()) {
        Ok(ciphertext) => Ok(ciphertext),
        Err(_) => Err(other_error("encrypt"))
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
        let mut count = 0;

        let name_cipher = b64::base64_encode(&encrypt_string(&cipher, name, &mut count)?);
        let mime_cipher = b64::base64_encode(&encrypt_string(&cipher, mime, &mut count)?);

        let new = Self {
            writer: writer,
            cipher: cipher,
            buffer: Vec::with_capacity(FORM_READ_BUFFER_SIZE * 2),
            count: count
        };

        Ok((new, encoded_key, name_cipher, mime_cipher))
    }

    pub fn finish(&mut self) -> Result<()> {
        // Make sure the file is terminated by two zero bytes
        self.writer.write(&0u16.to_be_bytes())?;
        Ok(())
    }
}

// `buffer` is a resizable buffer for intermediate data required by the
// encryption process.
pub fn encrypted_write<W>(
    plaintext: &[u8], buffer: &mut Vec<u8>, count: &mut u64, cipher: &Aes256Gcm, mut writer: W) -> Result<usize>
where W: Write
{
    if plaintext.is_empty() {
        return Ok(0);
    }

    if buffer.capacity() < plaintext.len() * 2 {
        buffer.reserve(plaintext.len() * 2 - buffer.len());
    }

    buffer.clear();
    buffer.extend_from_slice(plaintext);

    let nonce_bytes = nonce_bytes_from_count(count);
    *count += 1;

    match cipher.encrypt_in_place(Nonce::from_slice(&nonce_bytes), b"", buffer) {
        Ok(()) => {
            if buffer.len() <= MAX_CHUNK_SIZE {
                let size_prefix = (buffer.len() as u16).to_be_bytes();
                writer.write_all(&size_prefix)?;
                writer.write_all(&buffer)?;
                Ok(plaintext.len())
            } else {
                Err(other_error("Plaintext too large"))
            }
        },
        Err(_) => Err(other_error("encrypt_in_place"))
    }
}

impl Write for EncryptedFileWriter {
    fn write(&mut self, plaintext: &[u8]) -> Result<usize> {
        encrypted_write(plaintext, &mut self.buffer, &mut self.count, &self.cipher, &mut self.writer)
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
        let now = Local::now().naive_utc();
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

/*
// Basic wrapper around a buffered reader for a file.
pub struct FileReader {
    reader: BufReader<File>,
    expire_after: NaiveDateTime,
    is_completed: bool
}

impl FileReader {
    pub fn new(
            path: &PathBuf,
            start_index: u64,
            expire_after: NaiveDateTime,
            is_completed: bool) -> Result<Self>
    {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(start_index))?;
        let reader = BufReader::new(file);

        let new = Self {
            reader,
            expire_after,
            is_completed
        };

        Ok(new)
    }
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() == 0 {
            return Ok(0);
        }

        const ONE_SECOND: Duration = Duration::from_secs(1);

        let now = Local::now().naive_utc();
        if now > self.expire_after {
            Err(Error::new(ErrorKind::Other, "Upload expired during download"))
        } else {
            let bytes_read = self.reader.read(buf)?;

            // The upload might still be in progress while we're downloading,
            // pause and do another read.
            if bytes_read == 0 && !self.is_completed {
                std::thread::sleep(ONE_SECOND);
                self.reader.read(buf)
            } else {
                Ok(bytes_read)
            }
        }
    }
}
*/


pub struct AsyncFileReader {
    reader: tokio::io::BufReader<tokio::fs::File>,
    expire_after: NaiveDateTime,
    last_read_time: NaiveDateTime,
    is_completed: bool
}

impl AsyncFileReader {
    pub async fn new<P>(
        path: P, start_index: u64, expire_after: NaiveDateTime,
        is_completed: bool) -> Result<Self>
        where P: AsRef<Path>
    {
        let mut file = tokio::fs::File::open(path).await?;
        file.seek(SeekFrom::Start(start_index)).await?;

        Ok(Self {
            reader: tokio::io::BufReader::new(file),
            expire_after,
            last_read_time: Local::now().naive_utc(),
            is_completed
        })
    }
}

impl AsyncRead for AsyncFileReader {
    fn poll_read(
        mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>)
        -> Poll<Result<()>>
    {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        let now = Local::now().naive_utc();
        if now > self.expire_after {
            return Poll::Ready(Err(Error::new(ErrorKind::Other, "Upload expired during download")));
        }

        const ONE_SECOND: chrono::Duration = chrono::Duration::seconds(1);

        let buf_len = buf.filled().len();
        let pinned = pin!(&mut self.as_mut().reader);
        let f = pinned.poll_read(cx, buf);

        match f {
            Poll::Ready(Ok(())) => {
                if
                    // 0 bytes were read (EOF was reached)
                    buf.filled().len() == buf_len
                    // The upload may still be in progress while we're
                    // downloading
                    && !self.is_completed
                    // It's been at most 1 second since the last non-zero read
                    && now - self.last_read_time <= ONE_SECOND
                {
                    // The upload might still be in progress while we're
                    // downloading, so return pending to let the caller know
                    // to try again later.
                    // TODO: consider increasing this sleep duration
                    let s = sleep(tokio::time::Duration::from_secs(1));
                    match pin!(s).poll(cx) {
                        Poll::Ready(_) => Poll::Ready(Ok(())),
                        _ => Poll::Pending
                    }
                } else {
                    self.last_read_time = now;
                    f
                }
            },
            _ => f
        }
    }
}


pub struct AsyncEncryptedFileReader {
    reader: AsyncFileReader,
    cipher: Aes256Gcm,
    size_buf: [u8; 2],
    size_buf_len: usize,
    buffer: [u8; MAX_CHUNK_SIZE],
    buffer_len: usize,
    plaintext: Vec<u8>,
    plaintext_read_start: usize,
    count: u64,
    is_finished: bool
}

impl AsyncEncryptedFileReader {
    pub async fn new<P>(
        path: P,
        start_index: u64,
        expire_after: NaiveDateTime,
        is_completed: bool,
        key: &[u8],
        name_cipher: &[u8],
        mime_cipher: &[u8]) -> Result<(Self, String, String)>
        where P: AsRef<Path>
    {
        let key_slice = b64::base64_decode(key).ok_or(other_error("base64_decode"))?;
        let key = Key::from_slice(&key_slice);
        let cipher = Aes256Gcm::new(key);
        let mut count = 0;

        let name_cipher_decoded = b64::base64_decode(name_cipher)
            .ok_or(other_error("decrypting file name ciphertext"))?;
        let mime_cipher_decoded = b64::base64_decode(mime_cipher)
            .ok_or(other_error("decrypting file mime type ciphertext"))?;

        let name = decrypt_string(&cipher, &name_cipher_decoded, &mut count)?;
        let mime = decrypt_string(&cipher, &mime_cipher_decoded, &mut count)?;

        let new = Self {
            reader: AsyncFileReader::new(
                path, start_index, expire_after, is_completed).await?,
            cipher: cipher,
            size_buf: [0; 2],
            size_buf_len: 0,
            buffer: [0; MAX_CHUNK_SIZE],
            buffer_len: 0,
            plaintext: Vec::with_capacity(MAX_CHUNK_SIZE),
            plaintext_read_start: 0,
            count,
            is_finished: false
        };

        Ok((new, name, mime))
    }
}

// Helper function to try to fully read into the contents of `buf`.
// Reading 0 is considered an error, as we expect to be able to fill `buf`.
// NOTE: Exception to the above, if `buf` has length 0.
//
// Incomplete reads (where we read > 0 bytes, but less than the length of `buf`)
// are caught and we return a pending result instead (the waker is called to
// ensure the runtime keeps polling this reader to get more data).
//
// Returns the number of bytes read, and an optional poll result which is either
// an error, or a pending result.
fn poll_read_full<R>(
    reader: Pin<&mut R>, cx: &mut Context<'_>, buf: &mut[u8])
    -> (usize, Option<Poll<Result<()>>>)
    where R: AsyncRead
{
    if buf.len() == 0 {
        return (0, None);
    }

    let buf_len = buf.len();
    let mut readbuf = ReadBuf::new(buf);
    let f = reader.poll_read(cx, &mut readbuf);
    match f {
        Poll::Ready(Ok(())) => {
            let bytes_read = buf_len - readbuf.remaining();
            if bytes_read == 0 {
                // Unexpected EOF
                (0, Some(Poll::Ready(Err(other_error("Unexpected EOF")))))
            } else if bytes_read < buf_len {
                // Incomplete read
                cx.waker().wake_by_ref();
                (bytes_read, Some(Poll::Pending))
            } else {
                (bytes_read, None)
            }
        }
        _ => (0, Some(f))
    }
}

impl AsyncRead for AsyncEncryptedFileReader {
    fn poll_read(
        self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>)
        -> Poll<Result<()>>
    {
        // Async nonsense to be able to simultaneously mutably borrow separate
        // member variables.
        let s = self.get_mut();

        if s.is_finished {
            return Poll::Ready(Ok(()));
        }

        if s.plaintext.is_empty() {
            // Get the size of the next chunk to read (first 2 bytes)
            /* TODO: remove this, once I'm sure I don't need it
            let mut readbuf = ReadBuf::new(&mut s.size_buf[s.size_buf_len..2]);
            let bytes_remaining = readbuf.remaining();
            let f = pin!(&mut s.reader).poll_read(cx, &mut readbuf);
            match f {
                Poll::Ready(Ok(())) => {
                    s.size_buf_len += readbuf.filled().len();
                    if readbuf.remaining() == bytes_remaining {
                        return Poll::Ready(Err(other_error("Unexpected EOF")));
                    } else if readbuf.remaining() > 0 {
                        // If we didn't read the full 2 bytes, we need more data!
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                },
                _ => return f
            }
            */
            let (bytes_read, f) = poll_read_full(
                pin!(&mut s.reader), cx, &mut s.size_buf[s.size_buf_len..2]);
            s.size_buf_len += bytes_read;
            if let Some(f) = f { return f; }
            assert_eq!(s.size_buf_len, 2);

            let size_buf = [s.size_buf[0], s.size_buf[1]];
            let chunk_size = u16::from_be_bytes(size_buf) as usize;
            if chunk_size > MAX_CHUNK_SIZE {
                return Poll::Ready(Err(other_error("Ciphertext chunk too large")));
            } else if chunk_size == 0 {
                // A chunk size of 0 indicates the end of the file
                s.is_finished = true;
                return Poll::Ready(Ok(()));
            }

            // Read the full chunk
            let (bytes_read, f) = poll_read_full(
                pin!(&mut s.reader), cx, &mut s.buffer[s.buffer_len..chunk_size]);
            s.buffer_len += bytes_read;
            if let Some(f) = f { return f; }
            assert_eq!(s.buffer_len, chunk_size);

            // Decrypt the chunk
            let nonce_bytes = nonce_bytes_from_count(&s.count);
            s.count += 1;
            s.plaintext.extend_from_slice(&s.buffer[..s.buffer_len]);
            let decrypt_status = s.cipher.decrypt_in_place(
                Nonce::from_slice(&nonce_bytes), b"", &mut s.plaintext);
            if decrypt_status.is_err() {
                return Poll::Ready(Err(other_error("Decrypting ciphertext chunk failed")));
            }
        }

        // At this point, we have a plaintext
        assert!(!s.plaintext.is_empty());

        // Write as much of the plaintext as we can into the caller's ReadBuf
        let plaintext_remaining = &s.plaintext[s.plaintext_read_start..];
        let read_size = cmp::min(plaintext_remaining.len(), buf.remaining());
        buf.put_slice(&plaintext_remaining[..read_size]);
        s.plaintext_read_start += read_size;

        // If the entire plaintext was read, reset state variables to read
        // a new ciphertext chunk the next time this reader is polled.
        if read_size == plaintext_remaining.len() {
            s.plaintext.clear();
            s.buffer_len = 0;
            s.size_buf_len = 0;
            s.plaintext_read_start = 0;
        }

        Poll::Ready(Ok(()))
    }
}


fn decrypt_string(cipher: &Aes256Gcm, bytes: &[u8], count: &mut u64) -> Result<String> {
    let nonce_bytes = nonce_bytes_from_count(count);
    *count += 1;

    match cipher.decrypt(Nonce::from_slice(&nonce_bytes), bytes) {
        Ok(plaintext) => String::from_utf8(plaintext).or(Err(other_error("from_utf8"))),
        Err(_) => Err(other_error("decrypt"))
    }
}

/*
// Wrapper around FileReader. Decrypts its contents with the given key. Also
// decrypts the encrypted name and mime type of the file
pub struct EncryptedFileReader {
    reader: FileReader,
    cipher: Aes256Gcm,
    buffer: Vec<u8>,
    read_start: usize,
    read_end: usize,
    count: u64,
}

impl EncryptedFileReader {
    // Return the reader + the decrypted file name and decrypted mime type
    pub fn new(
        path: &PathBuf,
        start_index: u64,
        expire_after: NaiveDateTime,
        is_completed: bool,
        key: &[u8],
        name_cipher: &[u8],
        mime_cipher: &[u8]) -> Result<(Self, String, String)>
    {
        let key_slice = b64::base64_decode(key).ok_or(other_error("base64_decode"))?;
        let key = Key::from_slice(&key_slice);
        let cipher = Aes256Gcm::new(key);
        let mut count = 0;

        let name = decrypt_string(&cipher, &b64::base64_decode(name_cipher).ok_or(other_error("decrypt"))?, &mut count)?;
        let mime = decrypt_string(&cipher, &b64::base64_decode(mime_cipher).ok_or(other_error("decrypt"))?, &mut count)?;

        let new = Self {
            reader: FileReader::new(path, start_index, expire_after, is_completed)?,
            cipher: cipher,
            buffer: Vec::with_capacity(FORM_READ_BUFFER_SIZE * 2),
            read_start: 0,
            read_end: 0,
            count: count
        };

        Ok((new, name, mime))
    }
}

// `buffer` is a resizable buffer for intermediate data required by the
// decryption process. It is required here since the size of the plaintext
// we produce from a single ciphertext segment may exceed the size of the
// `plaintext` buffer, so it must be stored and returned in a subsequent call
// to this function.
pub fn encrypted_read<R>(
    plaintext: &mut[u8], buffer: &mut Vec<u8>, read_start: &mut usize,
    read_end: &mut usize, count: &mut u64, cipher: &Aes256Gcm, mut reader: R) -> Result<usize>
where R: Read
{
    if plaintext.is_empty() {
        return Ok(0);
    }

    if *read_start == *read_end {
        // if the buffer has no pending decrypted data

        let mut size_buf = 0u16.to_be_bytes();

        if let Err(e) = reader.read_exact(&mut size_buf) {
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
            return Err(other_error("Ciphertext chunk too large"));
        }

        buffer.resize(chunk_size, 0);
        reader.read_exact(buffer)?;

        let nonce_bytes = nonce_bytes_from_count(count);
        *count += 1;

        match cipher.decrypt_in_place(Nonce::from_slice(&nonce_bytes), b"", buffer) {
            Ok(()) => {
                let available_plaintext_len = buffer.len();
                let len = cmp::min(plaintext.len(), available_plaintext_len);

                plaintext[..len].copy_from_slice(&buffer[..len]);
                *read_start = len;
                *read_end = available_plaintext_len;

                Ok(len)
            },
            Err(_) => Err(other_error("decrypt_in_place"))
        }
    } else {
        // If there is remaining decrypted data that has yet to be sent
        let len = cmp::min(plaintext.len(), *read_end - *read_start);
        plaintext[..len].copy_from_slice(&buffer[*read_start..][..len]);
        *read_start += len;

        Ok(len)
    }
}

impl Read for EncryptedFileReader {
    fn read(&mut self, plaintext: &mut [u8]) -> Result<usize> {
        encrypted_read(
            plaintext, &mut self.buffer, &mut self.read_start,
            &mut self.read_end, &mut self.count, &self.cipher, &mut self.reader)
    }
}
*/

fn other_error(message: &'static str) -> Error {
    Error::new(ErrorKind::Other, message)
}

pub fn get_file_size<P>(file_path: P) -> Result<u64>
where P: AsRef<Path>
{
    let file_path = file_path.as_ref();

    File::open(file_path)
        .and_then(|f| f.metadata())
        .map(|m| m.len())
}

pub fn get_storage_size<P>(storage_dir: P) -> Result<usize>
where P: AsRef<Path>
{
    let storage_dir = storage_dir.as_ref();

    let mut storage_size = 0;

    for entry in storage_dir.read_dir()? {
        let upload = entry?.path().join("upload");

        if upload.exists() && upload.is_file() {
            if let Ok(size) = get_file_size(upload) {
                storage_size += size as usize;
            }
        }
    }

    Ok(storage_size)
}
