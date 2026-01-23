use std::fmt::Display;
use std::process;
use std::io::{self, Write, Error};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};


pub fn exit(msg: &str) -> ! {
    eprintln!("{}", msg);
    process::exit(1);
}

pub fn require<T, E>(result: Result<T, E>, msg: &str) -> T
where E: Display
{
    match result {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {}", msg, e);
            process::exit(1);
        }
    }
}


fn print_progress(progress: usize, total: usize) {
    // NOTE: maybe one day this will respond to the actual width of the terminal.
    // Today is not that day.
    const TOTAL_TICKS: usize = 20;
    if progress < total {
        let percent = 100 * progress / total;

        let filled_ticks = (TOTAL_TICKS * percent + 99) / 100;

        eprint!("[");
        for _ in 0..filled_ticks {
            eprint!("#");
        }
        for _ in filled_ticks..TOTAL_TICKS {
            eprint!(" ");
        }
        eprint!("] {}%\r", percent);

        io::stderr().flush().expect("Flushing stderr");
    } else {
        eprint!("{}\r", " ".repeat(TOTAL_TICKS * 2));
    }
}

pub async fn io_loop<R, W>(mut reader: R, mut writer: W, total_size: usize)
    -> Result<(), Error>
where R: AsyncReadExt + Unpin,
      W: AsyncWriteExt + Unpin
{
    let start_time = Instant::now();

    let mut progress = 0;
    let mut buf = [0; 1024 * 1024];
    loop {
        let bytes_read = reader.read(&mut buf).await?;
        if bytes_read == 0 {
            break;
        }

        progress += bytes_read;
        if progress > total_size {
            return Err(Error::other("Received more data than expected"));
        }

        // Show a progress bar if download is taking some time
        let elapsed = Instant::now().duration_since(start_time);
        if elapsed > Duration::from_secs(2) {
            print_progress(progress, total_size);
        }

        writer.write_all(&buf[..bytes_read]).await?;
    }

    Ok(())
}
