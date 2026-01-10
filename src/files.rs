use crate::b64;
use crate::random_bytes::*;
use crate::constants::*;
use crate::quotas::*;
use crate::storage_limit::*;

use std::io::{Result, Error, ErrorKind, SeekFrom};
use aead::Buffer;
use aes_gcm::aead::{AeadInPlace, Aead, NewAead};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use chrono::{NaiveDateTime, Local};
use std::cmp;
use std::future::Future;
use std::path::Path;
use std::pin::{pin, Pin};
use std::str;
use std::task::{Poll, Context};
use streaming_zip_async;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf, AsyncSeekExt, BufReader, BufWriter};
use tokio::time::sleep;

const MAX_CHUNK_SIZE: usize = FORM_READ_BUFFER_SIZE + 16;


fn nonce_bytes_from_count(count: u64) -> [u8; 12] {
    let mut nonce_bytes = [0; 12];
    nonce_bytes[..8].copy_from_slice(&u64::to_le_bytes(count));
    nonce_bytes
}

// Writers

pub enum WriterError {
    QuotaExceeded,
    StorageLimitExceeded,
    FileSizeLimitExceeded,
    NotSupported,
    Encryption,
    IO
}
impl From<Error> for WriterError {
    fn from(_: Error) -> Self {
        Self::IO
    }
}
type WriterResult<T> = std::result::Result<T, WriterError>;

pub trait Writer {
    async fn write<B>(&mut self, _bytes: B) -> WriterResult<()> where B: AsRef<[u8]> {
        Err(WriterError::NotSupported)
    }

    async fn start_new_file(&mut self, _name: &str) -> WriterResult<()> {
        Err(WriterError::NotSupported)
    }

    async fn finish_file(&mut self) -> WriterResult<()> {
        Err(WriterError::NotSupported)
    }

    async fn finish(self) -> WriterResult<()>
    where Self: Sized
    {
        Ok(())
    }

    // Needed right now because all the writers are built around an AsyncWrite
    // implementation, but we need to be able to report some additional error types.
    fn check(&mut self) -> WriterResult<()> {
        Ok(())
    }
}


// Plain file writer
impl<W> Writer for BufWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    async fn write<B>(&mut self, bytes: B) -> WriterResult<()>
    where B: AsRef<[u8]>
    {
        let result = self.write_all(bytes.as_ref()).await;
        self.get_mut().check()?;
        result?;
        Ok(())
    }

    async fn finish(mut self) -> WriterResult<()> {
        let result = self.flush().await;
        self.get_mut().check()?;
        result?;
        Ok(())
    }
}

pub struct AccountingFileWriter<W>
where W: AsyncWrite + Unpin
{
    writer: W,
    accounting_err: Option<WriterError>,
    max_upload_size: usize,
    bytes_written: usize,
    quota: Quota,
    storage_limit: StorageLimit
}

impl AccountingFileWriter<File> {
    pub async fn new<P>(
        path: P, max_upload_size: usize, quota: Quota,
        storage_limit: StorageLimit) -> Result<tokio::io::BufWriter<Self>>
    where P: AsRef<Path>
    {
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path).await?;

        let this = Self {
            writer: file,
            accounting_err: None,
            max_upload_size,
            bytes_written: 0,
            quota,
            storage_limit
        };

        Ok(tokio::io::BufWriter::new(this))
    }
}

impl<W> AsyncWrite for AccountingFileWriter<W>
where W: AsyncWrite + Unpin
{
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8])
        -> Poll<Result<usize>>
    {
        let this = self.as_mut().get_mut();
        let err = Poll::Ready(Err(Error::from(ErrorKind::Other)));

        // Check file size limit
        if
            this.bytes_written + buf.len() > this.max_upload_size
            && this.max_upload_size > 0
        {
            this.accounting_err = Some(WriterError::FileSizeLimitExceeded);
            return err;
        }

        // Check quota
        let mut quota = this.quota.lock();
        if !quota.check(buf.len()) {
            this.accounting_err = Some(WriterError::QuotaExceeded);
            return err;
        }

        // Check storage size
        let mut storage_limit = this.storage_limit.lock();
        if !storage_limit.check(buf.len()) {
            this.accounting_err = Some(WriterError::StorageLimitExceeded);
            return err;
        }

        let f = pin!(&mut this.writer).poll_write(cx, buf);
        match f {
            Poll::Ready(Ok(bytes_written)) => {
                this.bytes_written += bytes_written;
                quota.deduct(bytes_written);
                storage_limit.add(bytes_written);
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

impl<W> Writer for AccountingFileWriter<W>
where W: AsyncWrite + Unpin
{
    async fn write<B>(&mut self, bytes: B) -> WriterResult<()>
    where B: AsRef<[u8]>
    {
        let result = self.write_all(bytes.as_ref()).await;
        self.check()?;
        result?;
        Ok(())
    }

    async fn finish(mut self) -> WriterResult<()> {
        self.flush().await?;
        Ok(())
    }

    fn check(&mut self) -> WriterResult<()> {
        match self.accounting_err.take() {
            None => Ok(()),
            Some(e) => Err(e)
        }
    }
}


fn encrypt_string(cipher: &Aes256Gcm, string: &str, count: &mut u64) -> WriterResult<Vec<u8>> {
    let nonce_bytes = nonce_bytes_from_count(*count);
    *count += 1;

    match cipher.encrypt(Nonce::from_slice(&nonce_bytes), string.as_bytes()) {
        Ok(ciphertext) => Ok(ciphertext),
        Err(_) => Err(WriterError::Encryption)
    }
}

// Wrap a writer such that the data written is encrypted with the given key.
// Also encrypts the file name and mime type.
//
// The encrypted file is written as follows:
// - It is divided into segments of varying lengths
//   (but no longer than MAX_CHUNK_SIZE)
// - Each segment is prefixed by a 16-bit unsigned integer in big-endian byte
//   order which stores the length of the segment
// - The file ends with two zero bytes not belonging to any segment.
pub struct EncryptedFileWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    writer: W,
    cipher: Aes256Gcm,
    count: u64,

    buffer: [u8; 2 + MAX_CHUNK_SIZE],
    plaintext_len: usize,
    segment_len: usize,
    segment_write_start: usize,

    ending_bytes_written: u8,
}

impl<W> EncryptedFileWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    pub async fn new(writer: W, name: &str, mime: &str)
        -> WriterResult<(Self, Vec<u8>, Vec<u8>, Vec<u8>)>
    {
        let mut key_slice = [0; 32];
        random_bytes(&mut key_slice);

        let encoded_key = b64::base64_encode(&key_slice);
        let key = Key::from_slice(&key_slice);

        let cipher = Aes256Gcm::new(key);
        let mut count = 0;

        let name_cipher = b64::base64_encode(&encrypt_string(&cipher, name, &mut count)?);
        let mime_cipher = b64::base64_encode(&encrypt_string(&cipher, mime, &mut count)?);

        let this = Self {
            writer,
            cipher,
            buffer: [0; 2 + MAX_CHUNK_SIZE],
            segment_write_start: 0,
            plaintext_len: 0,
            segment_len: 0,
            count,
            ending_bytes_written: 0
        };
        
        Ok((this, encoded_key, name_cipher, mime_cipher))
    }

    // Returns Poll::Pending untill the full ciphertext is written
    fn encrypt_and_write_buffer(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<usize>> {
        let this = self.as_mut().get_mut();

        // Encrypt a new plaintext chunk
        if this.segment_len == 0 {
            let nonce_bytes = nonce_bytes_from_count(this.count);
            let nonce = Nonce::from_slice(&nonce_bytes);
            this.count += 1;

            let chunk = &mut this.buffer[2..];
            let mut buffer = FixedBuffer::new(chunk, this.plaintext_len);
            if this.cipher.encrypt_in_place(nonce, b"", &mut buffer).is_err() {
                return Poll::Ready(Err(other_error("encrypt_in_place")));
            } else if buffer.len() > MAX_CHUNK_SIZE {
                return Poll::Ready(Err(other_error("Plaintext too large")));
            }
            let ciphertext_len = buffer.len();

            let size_prefix = &mut this.buffer[..2];
            size_prefix.copy_from_slice(&(ciphertext_len as u16).to_be_bytes());

            this.segment_len = 2 + ciphertext_len;
        }

        // Write segment (size prefix + ciphertext chunk)
        let segment = &this.buffer[..this.segment_len][this.segment_write_start..];
        let f = pin!(&mut this.writer).poll_write(cx, segment);
        if let Poll::Ready(Ok(bytes_written)) = f {
            // Check if the entire segment was written
            self.segment_write_start += bytes_written;
            debug_assert!(self.segment_write_start <= self.segment_len);
            if self.segment_write_start >= self.segment_len {
                // Reset state variables
                self.segment_write_start = 0;
                self.plaintext_len = 0;
                self.segment_len = 0;
            }

            // Return "pending" until the full segment is written
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            f
        }
    }
}

impl<W> Writer for EncryptedFileWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    async fn write<B>(&mut self, bytes: B) -> WriterResult<()>
        where B: AsRef<[u8]>
    {
        let result = AsyncWriteExt::write(self, bytes.as_ref()).await;
        self.writer.check()?;
        result?;
        Ok(())
    }

    async fn finish(mut self) -> WriterResult<()> {
        // Encrypt and write any remaining plaintext in the buffer
        self.shutdown().await?;
        Ok(())
    }
}

impl<W> AsyncWrite for EncryptedFileWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8])
        -> Poll<Result<usize>>
    {
        if self.plaintext_len >= FORM_READ_BUFFER_SIZE {
            self.encrypt_and_write_buffer(cx)
        } else {
            let this = self.as_mut().get_mut();
            let chunk = &mut this.buffer[2..][..FORM_READ_BUFFER_SIZE];
            let remaining_chunk = &mut chunk[this.plaintext_len..];
            let take = std::cmp::min(remaining_chunk.len(), buf.len());
            remaining_chunk[..take].copy_from_slice(&buf[..take]);
            this.plaintext_len += take;
            Poll::Ready(Ok(take))
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
        if self.plaintext_len > 0 {
            // Write the final segment
            self.encrypt_and_write_buffer(cx).map_ok(|_| ())
        } else if self.ending_bytes_written < 2 {
            // Write a size prefix of 0 to indicate the end of the upload
            let ending_bytes = &[0, 0][self.ending_bytes_written as usize..];
            match pin!(&mut self.writer).poll_write(cx, ending_bytes) {
                Poll::Ready(Ok(0)) => Poll::Ready(Err(Error::from(ErrorKind::WriteZero))),
                Poll::Ready(Ok(bytes_written)) => {
                    self.ending_bytes_written += bytes_written as u8;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                },
                f => f.map_ok(|_| ())
            }
        } else {
            // Shut down the underlying writer
            pin!(&mut self.writer).poll_shutdown(cx)
        }
    }
}


pub struct EncryptedZipWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    writer: streaming_zip_async::Archive<EncryptedFileWriter<W>>
}

impl<W> EncryptedZipWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    pub async fn new(writer: W)
        -> WriterResult<(Self, Vec<u8>, Vec<u8>, Vec<u8>)>
    {
        let (writer, key, name, mime) = EncryptedFileWriter::new(writer, "", "application/zip").await?;

        let new = Self {
            writer: streaming_zip_async::Archive::new(writer)
        };

        Ok((new, key, name, mime))
    }
}

impl<W> Writer for EncryptedZipWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    async fn write<B>(&mut self, bytes: B) -> WriterResult<()>
        where B: AsRef<[u8]>
    {
        self.writer.append_data(bytes.as_ref()).await?;
        Ok(())
    }

    async fn start_new_file(&mut self, name: &str) -> WriterResult<()> {
        let now = Local::now().naive_utc();
        self.writer.start_new_file(
            name.to_owned().into_bytes(), now, true).await?;
        Ok(())
    }

    async fn finish_file(&mut self) -> WriterResult<()> {
        self.writer.finish_file().await?;
        Ok(())
    }

    async fn finish(mut self) -> WriterResult<()> {
        self.finish_file().await?;
        let inner_writer = self.writer.finish().await?;
        inner_writer.finish().await?;
        Ok(())
    }
}


// Readers

pub struct FileReader {
    reader: BufReader<File>,
    expire_after: NaiveDateTime,
    last_read_time: NaiveDateTime,
    is_completed: bool
}

impl FileReader {
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

impl AsyncRead for FileReader {
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


pub struct EncryptedFileReader<R>
where R: AsyncRead
{
    reader: R,
    cipher: Aes256Gcm,
    count: u64,
    is_finished: bool,

    size_buf: [u8; 2],
    size_buf_len: usize,

    buffer: [u8; MAX_CHUNK_SIZE],
    ciphertext_len: usize,
    plaintext_len: usize,
    plaintext_read_start: usize
}

impl EncryptedFileReader<FileReader> {
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
        let reader = FileReader::new(
            path, start_index, expire_after, is_completed).await?;
        Self::with_reader(reader, key, name_cipher, mime_cipher)
    }
}

impl<R> EncryptedFileReader<R>
where R: AsyncRead
{
    pub fn with_reader(
        reader: R,
        key: &[u8],
        name_cipher: &[u8],
        mime_cipher: &[u8]) -> Result<(Self, String, String)>
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
            reader,
            cipher: cipher,
            size_buf: [0; 2],
            size_buf_len: 0,
            buffer: [0; MAX_CHUNK_SIZE],
            ciphertext_len: 0,
            plaintext_len: 0,
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

impl<R> AsyncRead for EncryptedFileReader<R>
where R: AsyncRead + Unpin
{
    fn poll_read(
        self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>)
        -> Poll<Result<()>>
    {
        // Async nonsense to be able to simultaneously mutably borrow separate
        // member variables.
        let this = self.get_mut();

        if this.is_finished {
            return Poll::Ready(Ok(()));
        }

        if this.plaintext_len == 0 {
            // Get the size of the next chunk to read (first 2 bytes)
            let (bytes_read, f) = poll_read_full(
                pin!(&mut this.reader), cx, &mut this.size_buf[this.size_buf_len..2]);
            this.size_buf_len += bytes_read;
            if let Some(f) = f { return f; }
            assert_eq!(this.size_buf_len, 2);

            let size_buf = [this.size_buf[0], this.size_buf[1]];
            let chunk_size = u16::from_be_bytes(size_buf) as usize;
            if chunk_size > MAX_CHUNK_SIZE {
                return Poll::Ready(Err(other_error("Ciphertext chunk too large")));
            } else if chunk_size == 0 {
                // A chunk size of 0 indicates the end of the file
                this.is_finished = true;
                return Poll::Ready(Ok(()));
            }

            // Read the full chunk
            let (bytes_read, f) = poll_read_full(
                pin!(&mut this.reader), cx, &mut this.buffer[this.ciphertext_len..chunk_size]);
            this.ciphertext_len += bytes_read;
            if let Some(f) = f { return f; }
            assert_eq!(this.ciphertext_len, chunk_size);

            // Decrypt the chunk
            let nonce_bytes = nonce_bytes_from_count(this.count);
            this.count += 1;
            let ciphertext = &mut this.buffer[..this.ciphertext_len];
            let mut buffer = FixedBuffer::new(ciphertext, ciphertext.len());
            if this.cipher.decrypt_in_place(
                Nonce::from_slice(&nonce_bytes), b"", &mut buffer).is_err()
            {
                return Poll::Ready(Err(other_error("Decrypting ciphertext chunk failed")));
            }
            this.plaintext_len = buffer.len();
        }

        // At this point, we have a plaintext
        let plaintext = &this.buffer[..this.plaintext_len];
        debug_assert!(!plaintext.is_empty());

        // Write as much of the plaintext as we can into the caller's ReadBuf
        let plaintext_remaining = &plaintext[this.plaintext_read_start..];
        let read_size = cmp::min(plaintext_remaining.len(), buf.remaining());
        buf.put_slice(&plaintext_remaining[..read_size]);
        this.plaintext_read_start += read_size;

        // If the entire plaintext was read, reset state variables to read
        // a new ciphertext chunk the next time this reader is polled.
        if read_size == plaintext_remaining.len() {
            this.plaintext_len = 0;
            this.ciphertext_len = 0;
            this.size_buf_len = 0;
            this.plaintext_read_start = 0;
        }

        Poll::Ready(Ok(()))
    }
}


fn decrypt_string(cipher: &Aes256Gcm, bytes: &[u8], count: &mut u64) -> Result<String> {
    let nonce_bytes = nonce_bytes_from_count(*count);
    *count += 1;

    match cipher.decrypt(Nonce::from_slice(&nonce_bytes), bytes) {
        Ok(plaintext) => String::from_utf8(plaintext).or(Err(Error::from(ErrorKind::Other))),
        Err(_) => Err(Error::from(ErrorKind::Other))
    }
}

fn other_error(message: &'static str) -> Error {
    Error::new(ErrorKind::Other, message)
}

// An implementor of aead::Buffer backed by a fixed-size buffer
struct FixedBuffer<'a> {
    inner: &'a mut [u8],
    size: usize
}

impl<'a> FixedBuffer<'a> {
    fn new(inner: &'a mut [u8], size: usize) -> Self {
        Self { inner, size }
    }

    fn len(&self) -> usize {
        self.size
    }
}

impl<'a> AsRef<[u8]> for FixedBuffer<'a> {
    fn as_ref(&self) -> &'_ [u8] {
        &self.inner[..self.size]
    }
}

impl<'a> AsMut<[u8]> for FixedBuffer<'a> {
    fn as_mut(&mut self) -> &'_ mut [u8] {
        &mut self.inner[..self.size]
    }
}

impl<'a> Buffer for FixedBuffer<'a> {
    fn extend_from_slice(&mut self, s: &[u8]) -> std::result::Result<(), aead::Error> {
        if self.size + s.len() <= self.inner.len() {
            self.inner[self.size..][..s.len()].copy_from_slice(s);
            self.size += s.len();
            Ok(())
        } else {
            Err(aead::Error {})
        }
    }

    fn truncate(&mut self, len: usize) {
        debug_assert!(len <= self.inner.len());
        self.size = len;
    }
}
