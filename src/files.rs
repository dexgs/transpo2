use crate::b64;
use crate::random_bytes::*;
use crate::constants::*;
use crate::quotas::*;
use crate::storage_limit::*;

use std::io::{Result, Error, ErrorKind, SeekFrom};
use aead::Buffer;
use aes_gcm::aead::{AeadInPlace, NewAead};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use chrono::{NaiveDateTime, Local};
use std::cmp;
use std::boxed::Box;
use std::future::Future;
use std::path::Path;
use std::pin::{pin, Pin};
use std::str;
use std::task::{Poll, Context};
use streaming_zip_async;
use tokio::fs::File;
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
    ReadBuf, AsyncSeekExt, BufReader, BufWriter};
use tokio::time::{sleep, Sleep, Duration, Instant};

const SIZE_PREFIX_LEN: usize = 2;
const MAX_CHUNK_SIZE: usize = FORM_READ_BUFFER_SIZE + 16;

const MAX_FILE_NAME_CIPHERTEXT_SIZE: usize = 512;
const MAX_MIME_TYPE_CIPHERTEXT_SIZE: usize = 272;


// Writers

pub enum WriterError {
    QuotaExceeded,
    StorageLimitExceeded,
    FileSizeLimitExceeded,
    NotSupported,
    ValueTooLarge,
    Encryption,
    IO
}

impl From<Error> for WriterError {
    fn from(_: Error) -> Self {
        Self::IO
    }
}

impl From<aead::Error> for WriterError {
    fn from(_: aead::Error) -> Self {
        Self::Encryption
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

// Helper struct to handle the accounting for
// the various limits imposed on uploads
struct Accounting {
    max_upload_size: usize,
    bytes_written: usize,
    quota: Quota,
    storage_limit: StorageLimit
}

impl Accounting {
    fn check<'a>(&'a mut self, num_bytes: usize)
        -> std::result::Result<AccountingGuard<'a>, WriterError>
    {
        // Check max file size
        if self.bytes_written + num_bytes > self.max_upload_size {
            return Err(WriterError::FileSizeLimitExceeded);
        }

        // Check quota
        let mut quota = self.quota.lock();
        if !quota.check(num_bytes) {
            return Err(WriterError::QuotaExceeded);
        }

        // Check storage limit
        let storage_limit = self.storage_limit.lock();
        if !storage_limit.check(num_bytes) {
            return Err(WriterError::StorageLimitExceeded);
        }

        Ok(AccountingGuard {
            max_write_size: num_bytes,
            quota,
            storage_limit,
            bytes_written: &mut self.bytes_written
        })
    }
}

struct AccountingGuard<'a> {
    // The maximum write size for which this guard is valid
    max_write_size: usize,
    quota: QuotaGuard<'a>,
    storage_limit: StorageLimitGuard<'a>,
    bytes_written: &'a mut usize
}

impl<'a> AccountingGuard<'a> {
    // Commit writing `num_bytes` to accounting limits
    fn commit(mut self, num_bytes: usize) {
        debug_assert!(num_bytes <= self.max_write_size);
        *self.bytes_written += num_bytes;
        self.quota.deduct(num_bytes);
        self.storage_limit.add(num_bytes);
    }
}

pub struct AccountingWriter<W>
where W: AsyncWrite + Unpin
{
    writer: W,
    accounting_err: Option<WriterError>,
    accounting: Accounting
}

impl AccountingWriter<File> {
    pub async fn new<P>(
        path: P, max_upload_size: usize, quota: Quota,
        storage_limit: StorageLimit) -> Result<BufWriter<Self>>
    where P: AsRef<Path>
    {
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path).await?;

        let this = Self {
            writer: file,
            accounting_err: None,
            accounting: Accounting {
                max_upload_size,
                bytes_written: 0,
                quota,
                storage_limit
            }
        };

        Ok(tokio::io::BufWriter::new(this))
    }
}

impl<W> AsyncWrite for AccountingWriter<W>
where W: AsyncWrite + Unpin
{
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8])
        -> Poll<Result<usize>>
    {
        let this = self.as_mut().get_mut();

        let guard = match this.accounting.check(buf.len()) {
            Ok(guard) => guard,
            Err(e) => {
                this.accounting_err = Some(e);
                return Poll::Ready(Err(Error::from(ErrorKind::Other)));
            }
        };

        let f = pin!(&mut this.writer).poll_write(cx, buf);
        if let Poll::Ready(Ok(bytes_written)) = f {
            guard.commit(bytes_written);
        }
        f
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        pin!(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        pin!(&mut self.writer).poll_shutdown(cx)
    }
}

impl<W> Writer for AccountingWriter<W>
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


type CryptoResult<T> = std::result::Result<T, aead::Error>;

pub struct Crypto {
    cipher: Aes256Gcm,
    count: u64,
    nonce: Nonce<typenum::U12>
}

impl Crypto {
    pub fn from_slice(key_slice: &[u8]) -> Self {
        debug_assert_eq!(key_slice.len(), 32);
        let key = Key::from_slice(key_slice);
        let cipher = Aes256Gcm::new(key);

        Self {
            cipher,
            count: 0,
            nonce: Nonce::clone_from_slice(&[0; 12])
        }
    }

    fn increment_nonce(&mut self) -> () {
        self.nonce.as_mut_slice()[..8].copy_from_slice(&self.count.to_le_bytes());
        self.count += 1;
    }

    pub fn encrypt_string(&mut self, string: String) -> CryptoResult<Vec<u8>> {
        let mut string_bytes = string.into_bytes();
        string_bytes.reserve_exact(string_bytes.len() + 16);

        self.increment_nonce();
        self.cipher.encrypt_in_place(&self.nonce, b"", &mut string_bytes)?;
        Ok(string_bytes)
    }

    pub fn encrypt_bytes<'a>(
        &mut self, bytes: &'a mut [u8], plaintext_len: usize) -> CryptoResult<&'a [u8]>
    {
        self.increment_nonce();
        let mut buffer = FixedBuffer::new(bytes, plaintext_len);
        self.cipher.encrypt_in_place(&self.nonce, b"", &mut buffer)?;
        let buffer_len = buffer.len();
        Ok(&bytes[..buffer_len])
    }

    pub fn decrypt_string(&mut self, mut bytes: Vec<u8>) -> CryptoResult<String> {
        self.increment_nonce();
        self.cipher.decrypt_in_place(&self.nonce, b"", &mut bytes)?;
        match String::from_utf8(bytes) {
            Ok(s) => Ok(s),
            Err(_) => Err(aead::Error {})
        }
    }

    pub fn decrypt_bytes<'a>(&mut self, bytes: &'a mut [u8]) -> CryptoResult<&'a [u8]> {
        self.increment_nonce();
        let mut buffer = FixedBuffer::new(bytes, bytes.len());
        self.cipher.decrypt_in_place(&self.nonce, b"", &mut buffer)?;
        let buffer_len = buffer.len();
        Ok(&bytes[..buffer_len])
    }
}

fn crypto_result<T>(result: std::result::Result<T, aead::Error>) -> Result<T> {
    result.map_err(|_| Error::other("Encryption error"))
}

async fn write_header<W>(mut writer: W, data: &[u8], max_length: usize) -> WriterResult<()>
where W: AsyncWriteExt + Unpin
{
    if data.len() > max_length {
        return Err(WriterError::ValueTooLarge);
    }
    let size_prefix = (data.len() as u16).to_be_bytes();

    writer.write_all(&size_prefix).await?;
    writer.write_all(data).await?;

    Ok(())
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
pub struct EncryptedWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    writer: W,
    crypto: Crypto,

    buffer: [u8; SIZE_PREFIX_LEN + MAX_CHUNK_SIZE],
    plaintext_len: usize,
    segment_len: usize,
    segment_write_start: usize,

    ending_bytes_written: u8,
}

impl<W> EncryptedWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    pub async fn new(mut writer: W, name: String, mime: String)
        -> WriterResult<(Self, Vec<u8>)>
    {
        let mut key_slice = [0; 32];
        random_bytes(&mut key_slice);
        let encoded_key = b64::base64_encode(&key_slice);
        let mut crypto = Crypto::from_slice(&key_slice);

        write_header(
            &mut writer, &crypto.encrypt_string(name)?, MAX_FILE_NAME_CIPHERTEXT_SIZE).await?;
        write_header(
            &mut writer, &crypto.encrypt_string(mime)?, MAX_MIME_TYPE_CIPHERTEXT_SIZE).await?;

        Ok((Self::with_crypto(writer, crypto), encoded_key))
    }

    pub fn with_crypto(writer: W, crypto: Crypto) -> Self {
        Self {
            writer,
            crypto,
            buffer: [0; SIZE_PREFIX_LEN + MAX_CHUNK_SIZE],
            segment_write_start: 0,
            plaintext_len: 0,
            segment_len: 0,
            ending_bytes_written: 0
        }
    }

    // Returns Poll::Pending untill the full ciphertext is written
    fn encrypt_and_write_buffer(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<usize>> {
        let this = self.as_mut().get_mut();

        // Encrypt a new plaintext chunk
        if this.segment_len == 0 {
            let chunk = &mut this.buffer[SIZE_PREFIX_LEN..];
            let ciphertext = match this.crypto.encrypt_bytes(chunk, this.plaintext_len) {
                Ok(ciphertext) => ciphertext,
                Err(_) => return Poll::Ready(err("Encryption Error"))
            };
            if ciphertext.len() > MAX_CHUNK_SIZE {
                return Poll::Ready(err("Plaintext too large"));
            }
            let ciphertext_len = ciphertext.len();

            let size_prefix = &mut this.buffer[..SIZE_PREFIX_LEN];
            size_prefix.copy_from_slice(&(ciphertext_len as u16).to_be_bytes());

            this.segment_len = SIZE_PREFIX_LEN + ciphertext_len;
        }

        // Write segment (size prefix + ciphertext chunk)
        let segment = &this.buffer[..this.segment_len][this.segment_write_start..];
        let bytes_written = match pin!(&mut this.writer).poll_write(cx, segment) {
            Poll::Ready(Ok(bytes_written)) => bytes_written,
            f => return f
        };

        // Check if the entire segment was written
        self.segment_write_start += bytes_written;
        debug_assert!(self.segment_write_start <= self.segment_len);
        if self.segment_write_start == self.segment_len {
            // Reset state variables
            self.segment_write_start = 0;
            self.plaintext_len = 0;
            self.segment_len = 0;
        }

        // Return "pending" until the full segment is written
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

impl<W> Writer for EncryptedWriter<W>
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

impl<W> AsyncWrite for EncryptedWriter<W>
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
        } else if self.ending_bytes_written < SIZE_PREFIX_LEN as u8 {
            // Write a size prefix of 0 to indicate the end of the upload
            let ending_bytes = &[0; SIZE_PREFIX_LEN][self.ending_bytes_written as usize..];
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
    writer: streaming_zip_async::Archive<EncryptedWriter<W>>
}

impl<W> EncryptedZipWriter<W>
where W: Writer + AsyncWrite + Unpin
{
    pub async fn new(writer: W)
        -> WriterResult<(Self, Vec<u8>)>
    {
        let plaintext_name = String::from("");
        let plaintext_mime = String::from("application/zip");
        let (writer, key) = EncryptedWriter::new(
            writer, plaintext_name, plaintext_mime).await?;

        let new = Self {
            writer: streaming_zip_async::Archive::new(writer)
        };

        Ok((new, key))
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

pub struct Reader {
    reader: BufReader<File>,
    expire_after: NaiveDateTime,
    last_read_time: NaiveDateTime,
    sleep: Option<Pin<Box<Sleep>>>
}

impl Reader {
    pub async fn new<P>(
        path: P, start_index: u64, expire_after: NaiveDateTime,
        is_completed: bool) -> Result<Self>
        where P: AsRef<Path>
    {
        let mut file = File::open(path).await?;
        file.seek(SeekFrom::Start(start_index)).await?;

        let sleep = match is_completed {
            false => Some(Box::pin(sleep(Duration::ZERO))),
            true => None
        };

        Ok(Self {
            reader: BufReader::new(file),
            expire_after,
            last_read_time: Local::now().naive_utc(),
            sleep
        })
    }
}

impl AsyncRead for Reader {
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

        let initial_remaining = buf.remaining(); 
        match pin!(&mut self.reader).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {},
            f => return f
        }

        let this = self.as_mut().get_mut();
        if let Some(sleep) = this.sleep.as_mut() {
            let bytes_read = initial_remaining - buf.remaining();
            let time_since_last_read = now - this.last_read_time;
            // If we read zero bytes (reached EOF) and it's been *at most*
            // one second since the last non-zero read, then the upload
            // might still be in progress while we're downloading, so sleep
            // and try again later...
            if bytes_read == 0 && time_since_last_read <= ONE_SECOND {
                // TODO: consider increasing this sleep duration
                // (or replacing this with a better approach)
                sleep.as_mut().reset(Instant::now() + Duration::from_secs(1));
                if sleep.as_mut().poll(cx) == Poll::Pending {
                    return Poll::Pending;
                }
            }
        }

        this.last_read_time = now;
        Poll::Ready(Ok(()))
    }
}


async fn read_header<R>(mut reader: R, max_length: usize) -> Result<Vec<u8>>
where R: AsyncReadExt + Unpin
{
    let mut size_prefix = [0; SIZE_PREFIX_LEN];
    reader.read_exact(&mut size_prefix).await?;
    let size = u16::from_be_bytes(size_prefix) as usize;
    if size > max_length {
        return Err(Error::other("Value too large"));
    }

    let mut data = vec![0; size];
    reader.read_exact(&mut data).await?;
    Ok(data)
}

pub struct EncryptedReader<R>
where R: AsyncRead
{
    reader: R,
    crypto: Crypto,
    is_finished: bool,

    size_buf: [u8; SIZE_PREFIX_LEN],
    size_buf_len: usize,

    buffer: [u8; MAX_CHUNK_SIZE],
    ciphertext_len: usize,
    plaintext_len: usize,
    plaintext_read_start: usize
}

impl EncryptedReader<Reader> {
    pub async fn new<P>(
        path: P,
        start_index: u64,
        expire_after: NaiveDateTime,
        is_completed: bool,
        key: &[u8]) -> Result<(Self, String, String)>
    where P: AsRef<Path>
    {
        let reader = Reader::new(
            path, start_index, expire_after, is_completed).await?;
        Self::with_reader(reader, key).await
    }
}

impl<R> EncryptedReader<R>
where R: AsyncRead + Unpin
{
    pub async fn with_reader(mut reader: R, key: &[u8]) -> Result<(Self, String, String)> {
        let key_slice = b64::base64_decode(key).ok_or(error("base64_decode"))?;
        let mut crypto = Crypto::from_slice(&key_slice);

        let name_cipher = read_header(&mut reader, MAX_FILE_NAME_CIPHERTEXT_SIZE).await?;
        let name = crypto_result(crypto.decrypt_string(name_cipher))?;

        let mime_cipher = read_header(&mut reader, MAX_MIME_TYPE_CIPHERTEXT_SIZE).await?;
        let mime = crypto_result(crypto.decrypt_string(mime_cipher))?;

        let new = Self {
            reader,
            crypto,
            size_buf: [0; SIZE_PREFIX_LEN],
            size_buf_len: 0,
            buffer: [0; MAX_CHUNK_SIZE],
            ciphertext_len: 0,
            plaintext_len: 0,
            plaintext_read_start: 0,
            is_finished: false
        };

        Ok((new, name, mime))
    }
}

// Helper function to try to fully read into the contents of `buf`.
// Reading 0 is considered an error, as we expect to be able to fill `buf`.
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
    let buf_len = buf.len();
    let mut readbuf = ReadBuf::new(buf);
    match reader.poll_read(cx, &mut readbuf) {
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
        },
        f => (0, Some(f))
    }
}

impl<R> AsyncRead for EncryptedReader<R>
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

        if this.size_buf_len < SIZE_PREFIX_LEN {
            // Get the size of the next chunk to read
            let (bytes_read, f) = poll_read_full(
                pin!(&mut this.reader), cx, &mut this.size_buf[this.size_buf_len..]);
            this.size_buf_len += bytes_read;
            if let Some(f) = f { return f; }
        }

        if this.plaintext_len == 0 {
            assert_eq!(this.size_buf_len, this.size_buf.len());

            let chunk_size = u16::from_be_bytes(this.size_buf) as usize;
            if chunk_size > MAX_CHUNK_SIZE {
                return Poll::Ready(err("Ciphertext chunk too large"));
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
            let ciphertext = &mut this.buffer[..this.ciphertext_len];
            let plaintext = match this.crypto.decrypt_bytes(ciphertext) {
                Ok(plaintext) => plaintext,
                Err(_) => return Poll::Ready(err("Decrypting ciphertext chunk"))
            };
            this.plaintext_len = plaintext.len();
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


fn error(message: &'static str) -> Error {
    Error::other(message)
}

fn err<T>(message: &'static str) -> Result<T> {
    Err(error(message))
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
