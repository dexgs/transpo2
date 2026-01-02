use crate::b64;
use crate::random_bytes::*;
use crate::constants::*;

use std::io::{Result, Error, ErrorKind, SeekFrom};
use aes_gcm::aead::{AeadInPlace, Aead, NewAead};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use chrono::{NaiveDateTime, Local};
use std::cmp;
use std::fs::File;
use std::future::Future;
use std::path::Path;
use std::pin::{pin, Pin};
use std::str;
use std::task::{Poll, Context};
// use std::time::Duration;
use streaming_zip_async;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf, AsyncSeekExt};
use tokio::time::sleep;

const MAX_CHUNK_SIZE: usize = FORM_READ_BUFFER_SIZE + 16;


fn nonce_bytes_from_count(count: &u64) -> [u8; 12] {
    let mut nonce_bytes = [0; 12];
    nonce_bytes[..8].copy_from_slice(&u64::to_le_bytes(*count));
    nonce_bytes
}

// Writers

pub trait Writer {
    async fn write<B>(&mut self, bytes: B) -> Result<()> where B: AsRef<[u8]>;

    async fn start_new_file(&mut self, _name: &str) -> Result<()> {
        Err(Error::new(ErrorKind::InvalidInput, "This writer cannot start new files"))
    }

    async fn finish_file(&mut self) -> Result<()> {
        Err(Error::new(ErrorKind::InvalidInput, "This writer cannot finish files"))
    }

    async fn finish(self) -> Result<()> where Self: Sized {
        Ok(())
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
        pin!(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Result<()>>
    {
        pin!(&mut self.writer).poll_shutdown(cx)
    }
}

impl Writer for AsyncFileWriter {
    async fn write<B>(&mut self, bytes: B) -> Result<()>
        where B: AsRef<[u8]>
    {
        self.write_all(bytes.as_ref()).await?;
        Ok(())
    }
}


fn encrypt_string(cipher: &Aes256Gcm, string: &str, count: &mut u64) -> Result<Vec<u8>> {
    let nonce_bytes = nonce_bytes_from_count(count);
    *count += 1;

    match cipher.encrypt(Nonce::from_slice(&nonce_bytes), string.as_bytes()) {
        Ok(ciphertext) => Ok(ciphertext),
        Err(_) => Err(other_error("encrypt"))
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
pub struct AsyncEncryptedFileWriter {
    writer: AsyncFileWriter,
    cipher: Aes256Gcm,
    buffer: Vec<u8>,
    buffer_write_start: usize,
    size_prefix_start: u8,
    plaintext_len: usize,
    count: u64
}

impl AsyncEncryptedFileWriter {
    pub async fn new<P>(path: P, max_upload_size: usize, name: &str, mime: &str)
        -> Result<(Self, Vec<u8>, Vec<u8>, Vec<u8>)>
    where P: AsRef<Path>
    {
        let mut key_slice = [0; 32];
        random_bytes(&mut key_slice);

        let encoded_key = b64::base64_encode(&key_slice);
        let key = Key::from_slice(&key_slice);

        let cipher = Aes256Gcm::new(key);
        let writer = AsyncFileWriter::new(path, max_upload_size).await?;
        let mut count = 0;

        let name_cipher = b64::base64_encode(&encrypt_string(&cipher, name, &mut count)?);
        let mime_cipher = b64::base64_encode(&encrypt_string(&cipher, mime, &mut count)?);

        let this = Self {
            writer,
            cipher,
            buffer: Vec::with_capacity(FORM_READ_BUFFER_SIZE + 16),
            buffer_write_start: 0,
            size_prefix_start: 0,
            plaintext_len: 0,
            count
        };
        
        Ok((this, encoded_key, name_cipher, mime_cipher))
    }

    // Returns Poll::Pending untill the full ciphertext is written
    fn encrypt_and_write_buffer(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<usize>> {
        if self.plaintext_len == 0 {
            // Encrypt a new plaintext chunk

            assert_eq!(self.buffer_write_start, 0);
            assert_eq!(self.size_prefix_start, 0);
            //self.buffer.reserve_exact(buf.len() * 2);
            //self.buffer.extend_from_slice(&buf[..take]);
            self.plaintext_len = self.buffer.len();

            let nonce_bytes = nonce_bytes_from_count(&self.count);
            let nonce = Nonce::from_slice(&nonce_bytes);
            self.count += 1;

            let this = self.as_mut().get_mut();
            match this.cipher.encrypt_in_place(nonce, b"", &mut this.buffer) {
                Ok(()) => if this.buffer.len() > MAX_CHUNK_SIZE {
                    return Poll::Ready(Err(other_error("Plaintext too large")));
                },
                _ => {
                    return Poll::Ready(Err(other_error("encrypt_in_place")));
                }
            }
        }

        // Write chunk size
        if self.size_prefix_start < 2 {
            let size_prefix = (self.buffer.len() as u16).to_be_bytes();
            let this = self.as_mut().get_mut();
            let f = pin!(&mut this.writer).poll_write(cx, &size_prefix[this.size_prefix_start as usize..]);
            match f {
                Poll::Ready(Ok(bytes_written)) => {
                    self.size_prefix_start += bytes_written as u8;
                    // If the full size prefix was not written
                    if self.size_prefix_start < 2 {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                },
                _ => {
                    return f;
                }
            };
        }

        // Write ciphertext
        let this = self.as_mut().get_mut();
        let f = pin!(&mut this.writer).poll_write(cx, &this.buffer[this.buffer_write_start..]);
        match f {
            Poll::Ready(Ok(bytes_written)) => {
                // Check if the entire ciphertext was written
                self.buffer_write_start += bytes_written;
                if self.buffer_write_start >= self.buffer.len() {
                    // Report the write size to the caller as the size of the
                    // *plaintext* that was written
                    let ready = Poll::Ready(Ok(self.plaintext_len));

                    // reset state variables
                    self.buffer.clear();
                    self.buffer_write_start = 0;
                    self.size_prefix_start = 0;
                    self.plaintext_len = 0;

                    ready
                } else {
                    // Return "pending" until the full ciphertext is written
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
            _ => f
        }
    }

    pub async fn finish(&mut self) -> Result<()> {
        // Encrypt and write any remaining plaintext in the buffer
        self.flush().await?;
        // Write a chunk size of 0 to indicate the end of the file
        self.writer.write_all(&0u16.to_be_bytes()).await?;
        // Flush the underlying writer
        self.writer.flush().await?;
        Ok(())
    }
}

impl Writer for AsyncEncryptedFileWriter {
    async fn write<B>(&mut self, bytes: B) -> Result<()>
        where B: AsRef<[u8]>
    {
        AsyncWriteExt::write(self, bytes.as_ref()).await?;
        Ok(())
    }

    async fn finish(mut self) -> Result<()> {
        AsyncEncryptedFileWriter::finish(&mut self).await
    }
}

impl AsyncWrite for AsyncEncryptedFileWriter {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8])
        -> Poll<Result<usize>>
    {
        if self.buffer.len() >= FORM_READ_BUFFER_SIZE {
            match self.encrypt_and_write_buffer(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(_) => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        } else {
            let remaining = FORM_READ_BUFFER_SIZE - self.buffer.len();
            let take = std::cmp::min(remaining, buf.len());
            self.buffer.extend_from_slice(&buf[..take]);
            Poll::Ready(Ok(take))
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Result<()>>
    {
        // The buffer is cleared when the ciphertext has been written
        if self.buffer.len() > 0 {
            match self.encrypt_and_write_buffer(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(_) => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        } else {
            pin!(&mut self.writer).poll_flush(cx)
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Result<()>>
    {
        pin!(&mut self.writer).poll_shutdown(cx)
    }
}


pub struct AsyncEncryptedZipWriter {
    writer: streaming_zip_async::Archive<AsyncEncryptedFileWriter>
}

impl AsyncEncryptedZipWriter {
    pub async fn new<P>(path: P, max_upload_size: usize)
        -> Result<(Self, Vec<u8>, Vec<u8>, Vec<u8>)>
    where P: AsRef<Path>
    {
        let (inner_writer, key, name, mime) = AsyncEncryptedFileWriter::new(
            path, max_upload_size, "", "application/zip").await?;

        let new = Self {
            writer: streaming_zip_async::Archive::new(inner_writer)
        };

        Ok((new, key, name, mime))
    }
}

impl Writer for AsyncEncryptedZipWriter {
    async fn write<B>(&mut self, bytes: B) -> Result<()>
        where B: AsRef<[u8]>
    {
        self.writer.append_data(bytes.as_ref()).await?;
        Ok(())
    }

    async fn start_new_file(&mut self, name: &str) -> Result<()> {
        let now = Local::now().naive_utc();
        self.writer.start_new_file(
            name.to_owned().into_bytes(), now, true).await?;
        Ok(())
    }

    async fn finish_file(&mut self) -> Result<()> {
        self.writer.finish_file().await?;
        Ok(())
    }

    async fn finish(mut self) -> Result<()> {
        self.finish_file().await?;
        let inner_writer = self.writer.finish().await?;
        inner_writer.finish().await?;
        Ok(())
    }
}


// Readers

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
                (0, Some(Poll::Ready(Err(Error::from(ErrorKind::UnexpectedEof)))))
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
