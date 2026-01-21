use tokio::runtime::Builder;

mod upload;
mod settings;
mod util;


fn main() {
    let rt = Builder::new_current_thread()
        .enable_io()
        .build().expect("Starting async runtime");
    rt.block_on(async_main())
}


async fn async_main() {
    upload::main().await;
}
