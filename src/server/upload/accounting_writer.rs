use transpo2::files::*;

use crate::storage_limit::*;
use crate::quotas::*;

use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::fs::{OpenOptions, File};

use std::task::{Context, Poll};
use std::io::Result;
use std::path::Path;
use std::pin::{pin, Pin};


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
    accounting: Accounting
}

impl AccountingWriter<File> {
    pub async fn new<P>(
        path: P, max_upload_size: usize, quota: Quota,
        storage_limit: StorageLimit) -> Result<BufWriter<Self>>
    where P: AsRef<Path>
    {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path).await?;

        let this = Self {
            writer: file,
            accounting: Accounting {
                max_upload_size,
                bytes_written: 0,
                quota,
                storage_limit
            }
        };

        Ok(BufWriter::new(this))
    }
}

impl<W> AsyncWrite for AccountingWriter<W>
where W: AsyncWrite + Unpin
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8])
        -> Poll<Result<usize>>
    {
        let this = self.get_mut();
        let guard = this.accounting.check(buf.len())?;

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
    type Inner = W;

    async fn write<B>(&mut self, bytes: B) -> WriterResult<()>
    where B: AsRef<[u8]>
    {
        self.write_all(bytes.as_ref()).await?;
        Ok(())
    }
}

